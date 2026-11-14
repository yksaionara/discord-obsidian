//! Go Live runs in a worker process, `obsidian --stream-worker`, so screen
//! capture, video codecs and stream connections take memory only while
//! sharing or watching: the process exits when that stops and the memory
//! goes with it. Each worker shares this account's screen or shows one
//! other person's stream.
//!
//! Discord Obsidian starts the worker with one [`Job`] as a JSON line on its standard
//! input, and reads [`Report`]s back from its standard output. Once Discord
//! has started the stream, a [`Control::Send`] line hands the worker the
//! stream's server, and it sends the video there; a [`Control::Switch`] line
//! shares another screen, window or camera on the same stream. A `stop` line, or
//! closing its input, ends the job. Frames go to the UI through [`crate::frames`].

use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc as channel,
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use obsidian_discord::{
    rtp::Codec,
    stream::StreamServer,
    stream_connection::{self, Link, StreamReceiver, StreamSender},
};

use livekit::webrtc::{
    native::yuv_helper,
    video_frame::{I420Buffer, VideoBuffer},
};

use crate::{
    decode::Decoder,
    encode::{self, Effort, Encoder},
    frames::{Frames, Writer},
    stream_audio,
    voice::{Resolution, StreamQuality, StreamStats},
};

/// The argument that starts Discord Obsidian as a stream worker.
pub const ARGUMENT: &str = "--stream-worker";

/// The largest frame shared at 1080p. Discord's own client trials 1080p at
/// 30 fps for accounts without Nitro, up to 5 Mbit/s (its "frontier tuning"
/// experiment); larger screens are scaled down to it.
pub const SHARE_WIDTH: u32 = 1920;
pub const SHARE_HEIGHT: u32 = 1080;
/// How often the window's own preview of a shared screen updates, at most.
const PREVIEW_INTERVAL: Duration = Duration::from_millis(1000 / 30);
/// The largest preview of the shared screen the UI shows.
pub const PREVIEW_WIDTH: u32 = 1280;
pub const PREVIEW_HEIGHT: u32 = 720;
/// The largest picture shown of someone else's stream; larger ones are
/// scaled down to it.
pub const WATCH_WIDTH: u32 = 1920;
pub const WATCH_HEIGHT: u32 = 1080;
/// The largest preview Discord shows of a stream, and how often its client
/// sends a new one.
const THUMBNAIL_WIDTH: u32 = 512;
const THUMBNAIL_HEIGHT: u32 = 288;
const THUMBNAIL_EVERY: Duration = Duration::from_secs(5 * 60);
/// How long a shared stream whose connection ended waits for a server
/// Discord moved it to.
const MOVE_GRACE: Duration = Duration::from_secs(5);
/// How often a shared screen reports how it goes out, for the call's details.
const SENDING_EVERY: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Job {
    /// Captures `source`, writing frames that fit `width` by `height` to the
    /// frames file at `frames`.
    Share {
        source: Source,
        frames: PathBuf,
        width: u32,
        height: u32,
        fps: u32,
        /// How hard the encoder works on each frame.
        #[serde(default)]
        effort: Effort,
        /// Encoding on the GPU's encoder, where it has one.
        #[serde(default)]
        hardware: bool,
        /// Other applications' sound goes with it.
        #[serde(default = "sound_on")]
        sound: bool,
    },
    /// Receives and decodes the stream at `server`, writing pictures that
    /// fit `width` by `height` to the frames file at `frames`.
    Watch {
        server: StreamServer,
        frames: PathBuf,
        width: u32,
        height: u32,
        /// Where the stream's sound plays: the call's speakers, or the
        /// default output.
        #[serde(default)]
        speakers: Option<String>,
        /// Deafened in the call, so the stream's sound stays quiet.
        #[serde(default)]
        deafened: bool,
        /// How loud its sound plays; see [`crate::volume`].
        #[serde(default = "as_it_comes")]
        volume: f32,
    },
    /// Plays the video file at `file`, which may still be downloading,
    /// writing pictures that fit `width` by `height` to the frames file at
    /// `frames`; see [`crate::video_player`].
    Play {
        file: PathBuf,
        /// Its size, when known before it's downloaded.
        size: Option<u64>,
        frames: PathBuf,
        width: u32,
        height: u32,
        speakers: Option<String>,
        volume: f32,
    },
    /// Takes a picture of each screen in `screens`, then each window in
    /// `windows`, by the ids [`shareable`] listed them with, scaled down to
    /// fit `width` by `height`, for the share dialog to show. Each comes as a
    /// [`Report::Screen`].
    Glance {
        screens: Vec<u64>,
        #[serde(default)]
        windows: Vec<u64>,
        width: u32,
        height: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A screen or window the person picks in the system's dialog, or what
    /// they picked from [`shareable`] where there's none.
    Screen,
    /// A screen by the id [`shareable`] listed it with.
    Display(u64),
    /// A window by the id [`shareable`] listed it with.
    Window(u64),
    /// A camera by its id in [`crate::camera::list`], such as OBS's virtual
    /// camera.
    Camera(String),
    /// Moving colour bars, for testing without a screen to capture.
    TestPattern,
}

/// Whether Discord Obsidian lists screens and windows to share itself, where the system
/// has no dialog to pick one: everywhere but Linux.
pub const LISTS_SOURCES: bool = cfg!(not(target_os = "linux"));

/// A screen, window or camera [`shareable`] lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shareable {
    pub source: Source,
    pub title: String,
}

/// What Discord Obsidian can share: the screens, then the windows, when
/// [`LISTS_SOURCES`], then the cameras. Discord Obsidian's own windows aren't among
/// them, nor its console's.
pub fn shareable() -> Vec<Shareable> {
    let mut listed = screens_and_windows();
    listed.extend(crate::camera::list().into_iter().map(|device| Shareable {
        source: Source::Camera(device.id),
        title: device.name,
    }));
    listed
}

fn screens_and_windows() -> Vec<Shareable> {
    #[cfg(target_os = "linux")]
    {
        Vec::new()
    }
    #[cfg(not(target_os = "linux"))]
    {
        use livekit::webrtc::desktop_capturer::DesktopCaptureSourceType;
        let screens = sources(DesktopCaptureSourceType::Screen);
        let count = screens.len();
        let screens = screens
            .into_iter()
            .enumerate()
            .map(|(index, screen)| Shareable {
                source: Source::Display(screen.id()),
                title: if count == 1 {
                    "Entire screen".to_owned()
                } else {
                    format!("Screen {}", index + 1)
                },
            });
        // The console belongs to its host, so libwebrtc lists it as another
        // application's window.
        #[cfg(windows)]
        // SAFETY: no arguments; it only reads this process's console.
        let console = unsafe { windows::Win32::System::Console::GetConsoleWindow() }.0 as u64;
        #[cfg(not(windows))]
        let console = 0;
        let windows = sources(DesktopCaptureSourceType::Window)
            .into_iter()
            .filter_map(|window| {
                let title = window.title();
                (!title.trim().is_empty() && window.id() != console).then(|| Shareable {
                    source: Source::Window(window.id()),
                    title,
                })
            });
        screens.chain(windows).collect()
    }
}

#[cfg(not(target_os = "linux"))]
fn sources(
    kind: livekit::webrtc::desktop_capturer::DesktopCaptureSourceType,
) -> Vec<livekit::webrtc::desktop_capturer::CaptureSource> {
    livekit::webrtc::desktop_capturer::DesktopCapturer::new(
        livekit::webrtc::desktop_capturer::DesktopCapturerOptions::new(kind),
    )
    .map(|capturer| capturer.get_source_list())
    .unwrap_or_default()
}

/// What [`Source::Screen`] shares: the pick from [`shareable`] in the share
/// menu, until another. On Linux the pick may be [`Source::Screen`] itself,
/// which the system's dialog picks.
static PICKED: Mutex<Source> = Mutex::new(Source::Screen);

/// Shares `source` from now on where [`Source::Screen`] is asked for.
pub fn pick(source: Source) {
    *PICKED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = source;
}

/// `source`, with [`Source::Screen`] as what was picked.
fn picked(source: Source) -> Source {
    match source {
        Source::Screen => PICKED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
        source => source,
    }
}

/// What Discord Obsidian tells a running job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    /// Send the video to the server Discord started the stream on.
    Send(StreamServer),
    /// Share this screen or window in place of the one shared, once it gives
    /// a picture; on Linux, what the system's dialog picks. The stream goes
    /// on with the one shared until then, and when the dialog is cancelled.
    Switch(Source),
    /// Share at this size and rate from now on, as the stream goes on.
    Quality { width: u32, height: u32, fps: u32 },
    /// Share other applications' sound with the screen, or stop.
    Sound(bool),
    /// Play a watched stream's sound, or not, as being deafened changes.
    Hear(bool),
    /// Play a watched stream's or a video's sound this loud.
    Volume(f32),
    /// Play the video, from the start once it has ended.
    Play,
    Pause,
    /// Move to this many milliseconds into the video.
    Seek(u64),
    /// Draw the video's pictures to fit this.
    Size {
        width: u32,
        height: u32,
    },
    /// The video's file finished downloading, with this size; none when the
    /// download failed.
    Downloaded {
        size: Option<u64>,
    },
}

fn as_it_comes() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Report {
    /// The worker opened the frames file.
    Ready,
    /// Frames are flowing, at this size after scaling.
    Sharing { width: u32, height: u32 },
    /// Sharing: the video reaches the stream server. Watching: the first
    /// picture is on screen.
    Streaming,
    /// Sharing: how the stream goes out, once a second while it's live.
    Sending(StreamStats),
    /// Sharing: a preview of the stream for Discord to show, a JPEG as a
    /// data URI. The first comes once a picture isn't black, then one every
    /// 5 minutes, as Discord's client sends them.
    Preview { thumbnail: String },
    /// The video file opened: how long it plays, when it says, its
    /// pictures' size, and whether its sound plays.
    Opened {
        duration_ms: Option<u64>,
        width: u32,
        height: u32,
        sound: bool,
    },
    /// The video played to its end, and waits there.
    Finished,
    /// A picture of the screen [`shareable`] listed by `id`, for
    /// [`Job::Glance`]: RGBA pixels, `width` by `height`, of a screen, or a
    /// window when `window`.
    Screen {
        id: u64,
        #[serde(default)]
        window: bool,
        width: u32,
        height: u32,
        #[serde(with = "base64_pixels")]
        rgba: Vec<u8>,
    },
    /// The job ended: without an error when it was told to stop.
    Ended { error: Option<String> },
}

fn sound_on() -> bool {
    true
}

/// Runs this process as a stream worker until its job ends.
pub fn run() -> Result<()> {
    // The stream connection uses TLS; nothing else here has started it.
    let _ = crate::init_tls();
    let mut output = std::io::stdout();
    serve(Box::new(BufReader::new(std::io::stdin())), &mut |report| {
        // Discord Obsidian is gone if this fails; the job ends when its input closes.
        let _ = writeln!(
            output,
            "{}",
            serde_json::to_string(&report).unwrap_or_default()
        );
        let _ = output.flush();
    });
    Ok(())
}

fn serve(mut input: Box<dyn BufRead + Send>, report: &mut dyn FnMut(Report)) {
    let mut line = String::new();
    let job = match input.read_line(&mut line) {
        Ok(_) => {
            serde_json::from_str::<Job>(&line).map_err(|error| anyhow!("unknown job: {error}"))
        }
        Err(error) => Err(error.into()),
    };
    let stop = Arc::new(AtomicBool::new(false));
    let stopper = stop.clone();
    let (controls, received) = channel::channel();
    // Reading stdin blocks, so a thread waits for controls, the stop line or the end.
    std::thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else { break };
            if line.trim() == "stop" {
                break;
            }
            match serde_json::from_str::<Control>(&line) {
                Ok(control) => {
                    let _ = controls.send(control);
                }
                Err(error) => tracing::warn!(%error, "unknown stream worker control"),
            }
        }
        stopper.store(true, Ordering::Relaxed);
    });
    let result = job.and_then(|job| match job {
        Job::Share {
            source,
            frames,
            width,
            height,
            fps,
            effort,
            hardware,
            sound,
        } => {
            let fps = fps.clamp(1, crate::voice::MAX_FPS);
            let sink = Sink {
                writer: Writer::open(&frames)?,
                controls: received,
                server: None,
                sound: sound && source != Source::TestPattern,
                video: None,
                fps,
                effort,
                hardware,
                started: Instant::now(),
                streaming: false,
                preview_due: None,
                thumbnail: None,
                lost: None,
                previewed: None,
                requality: None,
                reopen: false,
            };
            report(Report::Ready);
            #[cfg(windows)]
            let _precise = Precise::begin();
            match source {
                Source::TestPattern => test_pattern(sink, width, height, &stop, report),
                source => capture_screen(source, sink, width, height, &stop, report),
            }
        }
        Job::Watch {
            server,
            frames,
            width,
            height,
            speakers,
            deafened,
            volume,
        } => {
            let writer = Writer::open(&frames)?;
            report(Report::Ready);
            let hearing = Arc::new(AtomicBool::new(!deafened));
            let sound = Sound {
                speakers,
                hearing,
                volume: Arc::new(stream_audio::Volume::new(volume)),
                controls: received,
            };
            watch(server, writer, (width, height), sound, &stop, report)
        }
        Job::Play {
            file,
            size,
            frames,
            width,
            height,
            speakers,
            volume,
        } => {
            let writer = Writer::open(&frames)?;
            report(Report::Ready);
            let job = crate::video_player::Playing {
                path: file,
                size,
                width,
                height,
                sound: true,
                speakers,
                volume,
            };
            crate::video_player::play(job, writer, received, &stop, report)
        }
        Job::Glance {
            screens,
            windows,
            width,
            height,
        } => {
            report(Report::Ready);
            glance(&screens, false, width, height, &stop, report);
            glance(&windows, true, width, height, &stop, report);
            Ok(())
        }
    });
    report(Report::Ended {
        error: result.err().map(|error| format!("{error:#}")),
    });
}

/// While sharing on Windows: timers to the millisecond for the packet pacer
/// and the frame schedule, which Windows otherwise rounds up to its 15.6 ms
/// tick in this process, whatever a game sets in its own; and the capture
/// thread ahead of others for the processor, so a game using every core
/// doesn't make the stream stutter. Undone when sharing ends.
#[cfg(windows)]
struct Precise;

#[cfg(windows)]
impl Precise {
    fn begin() -> Self {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL,
        };
        // SAFETY: plain calls on this process's timer and this thread.
        unsafe {
            windows::Win32::Media::timeBeginPeriod(1);
            if let Err(error) = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) {
                tracing::debug!(%error, "the capture thread keeps its priority");
            }
        }
        Self
    }
}

#[cfg(windows)]
impl Drop for Precise {
    fn drop(&mut self) {
        // SAFETY: ends the period `begin` started.
        unsafe {
            windows::Win32::Media::timeEndPeriod(1);
        }
    }
}

/// Where each frame goes: the UI's preview, then Discord once the stream
/// has a server.
struct Sink {
    writer: Writer,
    controls: channel::Receiver<Control>,
    /// The server to send to from the next frame, which gives the stream its size.
    server: Option<StreamServer>,
    /// Whether to share other applications' sound too: not for the test
    /// pattern.
    sound: bool,
    video: Option<Video>,
    fps: u32,
    effort: Effort,
    hardware: bool,
    started: Instant,
    streaming: bool,
    /// When the stream's next preview is due: from when it starts.
    preview_due: Option<Instant>,
    /// A preview made but not reported yet.
    thumbnail: Option<String>,
    /// When the stream connection ended, while Discord may still name a
    /// server it moved the stream to.
    lost: Option<Instant>,
    /// When the window's preview last got a frame.
    previewed: Option<Instant>,
    /// A size asked for by [`Control::Quality`] that the capture hasn't taken
    /// yet, and whether the encoder opens again for its new rate.
    requality: Option<(u32, u32)>,
    reopen: bool,
}

struct Video {
    sender: StreamSender,
    encoder: Encoder,
    /// Other applications' sound, while sharing a screen.
    sound: Option<stream_audio::Capture>,
    /// When the stream last reported how it goes out, and the bytes it had
    /// sent then.
    measured: (Instant, u64),
    /// When the encoder last gave a keyframe.
    keyframed: Option<Instant>,
}

impl Video {
    /// How the stream goes out, with the kbit/s sent since it last said.
    fn sending(&mut self, fps: u32) -> StreamStats {
        let sent = self.sender.sent();
        let (since, before) = self.measured;
        let bits = sent.video_bytes.saturating_sub(before) * 8;
        let milliseconds = since.elapsed().as_millis().max(1) as u64;
        self.measured = (Instant::now(), sent.video_bytes);
        let (width, height) = self.encoder.size();
        StreamStats {
            engine: self.encoder.engine().to_owned(),
            codec: self.encoder.codec().name().to_owned(),
            width,
            height,
            fps,
            target_kbps: self.encoder.bitrate(),
            sent_kbps: (bits / milliseconds) as u32,
            keyframe_requests: sent.keyframe_requests,
            resent: sent.resent,
        }
    }
}

/// The codecs only a GPU encodes, most wanted first, offered before H264
/// where the GPU's encoder for them opens.
const GPU_CODECS: [Codec; 2] = [Codec::Av1, Codec::H265];

/// The size a GPU encoder is opened at only to see that it opens. One that
/// then doesn't open at the stream's size is withdrawn when Discord chooses
/// it.
const PROBE: (u32, u32) = (640, 360);

fn log_encoder(encoder: &Encoder) {
    let (width, height) = encoder.size();
    tracing::info!(
        engine = encoder.engine(),
        codec = ?encoder.codec(),
        width,
        height,
        "encoding the stream"
    );
}

/// The kbit/s to aim for, as Discord's client does: its cap for frames of
/// `height` lines at `fps`, or the connection's lower `target` while packets
/// are lost on the way, but at least the 500 kbit/s it gives a screen.
fn kbps(height: u32, fps: u32, target: Option<u64>) -> u32 {
    let cap = stream_connection::max_bitrate(height, fps) / 1000;
    target.map_or(cap, |target| {
        (target / 1000).clamp(500, u64::from(cap)) as u32
    })
}

/// Whether a frame of `size` is near enough the encoder's `encoding` to be
/// scaled to it rather than open the encoder again: within 4% each way, as
/// a window that gains or loses its border or a row gives. Opening again
/// costs a keyframe and a stall, and a window flipping between two such
/// sizes had the encoder open every few seconds.
fn near(size: (u32, u32), encoding: (u32, u32)) -> bool {
    let close = |a: u32, b: u32| a.abs_diff(b) * 25 <= b;
    close(size.0, encoding.0) && close(size.1, encoding.1)
}

impl Sink {
    /// Takes a frame at the size it's sent: the preview shows it, scaled
    /// down to fit, and Discord gets it once the stream has a server.
    fn frame(&mut self, picture: &mut I420Buffer) -> Result<()> {
        let (width, height) = (picture.width(), picture.height());
        // The preview in the window updates at most every PREVIEW_INTERVAL:
        // scaling and converting every frame of a 240 fps stream would cost
        // more than encoding it. Discord's preview picture comes from the
        // same frames.
        let now = Instant::now();
        let preview_now = self
            .previewed
            .is_none_or(|at| now.duration_since(at) >= PREVIEW_INTERVAL);
        let thumbnail_due = self.preview_due.is_some_and(|due| due <= now);
        if preview_now || thumbnail_due {
            self.previewed = Some(now);
            let preview = fit(width, height, PREVIEW_WIDTH, PREVIEW_HEIGHT);
            let mut smaller = (preview != (width, height))
                .then(|| picture.scale(preview.0 as i32, preview.1 as i32));
            let shown = match smaller.as_mut() {
                Some(smaller) => smaller,
                None => &mut *picture,
            };
            self.writer
                .write(preview.0, preview.1, |pixels| to_rgba(shown, pixels));
            self.preview_picture(shown, thumbnail_due);
        }
        self.send_frame(picture)
    }

    /// Makes Discord's preview picture from `shown` when one is due.
    fn preview_picture(&mut self, shown: &mut I420Buffer, due: bool) {
        if due && !is_black(shown) {
            match thumbnail(shown) {
                Ok(thumbnail) => self.thumbnail = Some(thumbnail),
                Err(error) => {
                    tracing::warn!(error = format!("{error:#}"), "could not make a preview")
                }
            }
            self.preview_due = Some(Instant::now() + THUMBNAIL_EVERY);
        }
    }

    /// Encodes `picture` and sends it, once the stream has a server.
    fn send_frame(&mut self, picture: &mut I420Buffer) -> Result<()> {
        // A frame a few pixels off the stream's size is scaled to it, unless
        // a new quality asked for another size.
        let mut rescaled = None;
        let picture = match self.video.as_ref().map(|video| video.encoder.size()) {
            Some(encoding)
                if !self.reopen
                    && (picture.width(), picture.height()) != encoding
                    && near((picture.width(), picture.height()), encoding) =>
            {
                rescaled.insert(picture.scale(encoding.0 as i32, encoding.1 as i32))
            }
            _ => picture,
        };
        let (width, height) = (picture.width(), picture.height());
        if let Some(server) = self.server.take() {
            let (offers, encoder) = self.offers(width, height)?;
            let sender = StreamSender::start(server, width, height, self.fps, offers)?;
            let sound = if self.sound {
                stream_audio::Capture::start(sender.sound())
                    .inspect_err(|error| {
                        tracing::warn!(error = format!("{error:#}"), "sharing without sound")
                    })
                    .ok()
            } else {
                None
            };
            self.video = Some(Video {
                sender,
                encoder,
                sound,
                measured: (Instant::now(), 0),
                keyframed: None,
            });
        }
        let reopen = std::mem::take(&mut self.reopen);
        let Some(video) = self.video.as_ref() else {
            return Ok(());
        };
        // Discord's choice of codec changes as viewers come and go.
        let codec = video.sender.codec().unwrap_or(video.encoder.codec());
        let resized = video.encoder.size() != (width, height);
        if resized || reopen || codec != video.encoder.codec() {
            let opened = self.encoder(codec, width, height);
            let video = self.video.as_mut().expect("checked above");
            match opened {
                Ok(encoder) => {
                    video.encoder = encoder;
                    if resized {
                        video.sender.resize(width, height);
                    }
                    if reopen {
                        video.sender.set_fps(self.fps);
                    }
                }
                // A GPU codec that opened at the start may not open again,
                // or at another size: Discord then chooses another, which
                // the next frame opens.
                Err(error) if codec != stream_connection::SEND_CODEC => {
                    tracing::warn!(
                        error = format!("{error:#}"),
                        ?codec,
                        "the GPU's encoder didn't open again"
                    );
                    video.sender.withdraw(codec);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        let Some(video) = self.video.as_mut() else {
            return Ok(());
        };
        let wanted = kbps(height, self.fps, Some(video.sender.target_bitrate()));
        let aimed = video.encoder.bitrate();
        // Not on every wobble, which would churn the rate control.
        if wanted.abs_diff(aimed) > aimed / 10 {
            match video.encoder.set_bitrate(wanted) {
                Ok(()) => tracing::debug!(kbps = wanted, "the encoder's bitrate"),
                Err(error) => {
                    tracing::warn!(error = format!("{error:#}"), "could not change the bitrate")
                }
            }
        }
        // The network is behind: skip this frame rather than queue it,
        // which keeps what viewers decode whole.
        if video.sender.backlogged() {
            return Ok(());
        }
        let pts = self.started.elapsed().as_micros() as u64 * encode::CLOCK / 1_000_000;
        // A keyframe asked for within a second of the last waits its turn.
        let keyframe = video
            .keyframed
            .is_none_or(|at| at.elapsed() >= stream_connection::KEYFRAME_GAP)
            && video.sender.take_keyframe_request();
        let codec = video.encoder.codec();
        match video.encoder.encode_i420(picture, pts, keyframe) {
            Ok(Some(frame)) => {
                if frame.keyframe {
                    video.keyframed = Some(Instant::now());
                }
                video.sender.send(stream_connection::Frame {
                    data: frame.data.to_vec(),
                    codec,
                    timestamp: pts as u32,
                    keyframe: frame.keyframe,
                })
            }
            Ok(None) => {}
            // Only the GPU encodes AV1 and H265: Discord chooses another
            // codec instead, which the next frame encodes.
            Err(error) if codec != stream_connection::SEND_CODEC => {
                tracing::warn!(
                    error = format!("{error:#}"),
                    ?codec,
                    "the GPU's encoder failed"
                );
                video.sender.withdraw(codec);
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// The codecs to offer Discord, most wanted first, and an encoder for
    /// the first: AV1, then H265, where the GPU's encoder for each opens,
    /// then H264. Discord streams the first every viewer decodes. Each is
    /// opened to see that it opens: the first at the stream's size, and
    /// kept; those after it at [`PROBE`]'s, since what an encoder took
    /// while open doesn't all come back when it closes.
    fn offers(&self, width: u32, height: u32) -> Result<(Vec<Codec>, Encoder)> {
        let mut offers = vec![];
        let mut first = None;
        for codec in GPU_CODECS.iter().copied().filter(|_| self.hardware) {
            let (width, height) = match first {
                None => (width, height),
                Some(_) => (width.min(PROBE.0), height.min(PROBE.1)),
            };
            if let Ok(encoder) = self.open(codec, width, height) {
                offers.push(codec);
                first.get_or_insert(encoder);
            }
        }
        offers.push(stream_connection::SEND_CODEC);
        let encoder = match first {
            Some(encoder) => encoder,
            None => self.open(stream_connection::SEND_CODEC, width, height)?,
        };
        log_encoder(&encoder);
        Ok((offers, encoder))
    }

    /// An encoder for the stream in `codec`, as [`Self::open`] opens it.
    fn encoder(&self, codec: Codec, width: u32, height: u32) -> Result<Encoder> {
        let encoder = self.open(codec, width, height)?;
        log_encoder(&encoder);
        Ok(encoder)
    }

    /// An encoder in `codec`, at the bitrate the stream aims for: where the
    /// rate control starts, before the stream connects.
    fn open(&self, codec: Codec, width: u32, height: u32) -> Result<Encoder> {
        let target = self
            .video
            .as_ref()
            .map_or(stream_connection::START_BITRATE, |video| video.sender.target_bitrate());
        Encoder::new(
            codec,
            width,
            height,
            self.fps,
            kbps(height, self.fps, Some(target)),
            self.effort,
            self.hardware,
        )
    }

    /// Takes Discord Obsidian's controls, reports the stream starting, and fails once
    /// its connection ended. Gives the source a [`Control::Switch`] asked for.
    fn check(&mut self, report: &mut dyn FnMut(Report)) -> Result<Option<Source>> {
        let mut switch = None;
        while let Ok(control) = self.controls.try_recv() {
            match control {
                Control::Send(server) => self.server = Some(server),
                Control::Switch(source) => switch = Some(source),
                Control::Quality { width, height, fps } => {
                    self.fps = fps.clamp(1, crate::voice::MAX_FPS);
                    self.requality = Some((width, height));
                    self.reopen = true;
                }
                Control::Sound(on) => {
                    self.sound = on;
                    if let Some(video) = self.video.as_mut() {
                        if !on {
                            video.sound = None;
                        } else if video.sound.is_none() {
                            video.sound = stream_audio::Capture::start(video.sender.sound())
                                .inspect_err(|error| {
                                    tracing::warn!(error = format!("{error:#}"), "sharing without sound")
                                })
                                .ok();
                        }
                    }
                }
                _ => {}
            }
        }
        match self.video.as_ref().map(|video| video.sender.link()) {
            Some(Link::Connected) if !self.streaming => {
                self.streaming = true;
                self.preview_due = Some(Instant::now());
                report(Report::Streaming);
            }
            // Discord ends the connection when it moves the stream to
            // another server, which then arrives as another Send.
            Some(Link::Ended(reason)) => {
                let lost = *self.lost.get_or_insert_with(Instant::now);
                if lost.elapsed() >= MOVE_GRACE {
                    bail!("Lost the stream connection: {reason}");
                }
            }
            _ => self.lost = None,
        }
        if let Some(thumbnail) = self.thumbnail.take() {
            report(Report::Preview { thumbnail });
        }
        if let Some(video) = self.video.as_mut()
            && self.streaming
            && video.measured.0.elapsed() >= SENDING_EVERY
        {
            report(Report::Sending(video.sending(self.fps)));
        }
        Ok(switch)
    }
}

/// Whether a picture is black, as a screen is before what's shared draws:
/// Discord's client waits for one that isn't before sending a preview.
fn is_black(picture: &I420Buffer) -> bool {
    let (y, _, _) = picture.data();
    // Black's luma is 16; some of a picture of anything is brighter.
    y.iter().step_by(7).all(|&luma| luma <= 24)
}

/// A preview of a picture as Discord's client makes one: scaled down to fit
/// 512 by 288, a JPEG as a data URI.
fn thumbnail(picture: &mut I420Buffer) -> Result<String> {
    use base64::Engine;
    use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
    let (width, height) = fit(
        picture.width(),
        picture.height(),
        THUMBNAIL_WIDTH,
        THUMBNAIL_HEIGHT,
    );
    let small = picture.scale(width as i32, height as i32);
    let mut rgba = vec![0; (width * height * 4) as usize];
    to_rgba(&small, &mut rgba);
    let rgb: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    let mut jpeg = vec![];
    JpegEncoder::new_with_quality(&mut jpeg, 92).encode(
        &rgb,
        width,
        height,
        ExtendedColorType::Rgb8,
    )?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
}

/// How a watched stream's sound plays, and Discord Obsidian's word on hearing it.
struct Sound {
    speakers: Option<String>,
    hearing: Arc<AtomicBool>,
    volume: Arc<stream_audio::Volume>,
    controls: channel::Receiver<Control>,
}

/// Shows someone's stream: whole frames from the connection, decoded, then
/// scaled down if they don't fit, and plays its sound.
fn watch(
    server: StreamServer,
    mut writer: Writer,
    (max_width, max_height): (u32, u32),
    sound: Sound,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<()> {
    let mut receiver = StreamReceiver::start(server, crate::decode::DECODES.to_vec())?;
    let _player = receiver.take_sounds().and_then(|sounds| {
        stream_audio::Player::start(
            sounds,
            sound.speakers,
            sound.hearing.clone(),
            sound.volume.clone(),
        )
        .inspect_err(|error| tracing::warn!(error = format!("{error:#}"), "watching without sound"))
        .ok()
    });
    let mut decoder: Option<Decoder> = None;
    // A frame that doesn't fit scales through this, a picture of the size
    // shown; one that does goes straight into the shared frame.
    let mut scratch = vec![];
    let mut shown = false;
    let mut failures = 0u64;
    while !stop.load(Ordering::Relaxed) {
        while let Ok(control) = sound.controls.try_recv() {
            match control {
                Control::Hear(hearing) => sound.hearing.store(hearing, Ordering::Relaxed),
                Control::Volume(volume) => sound.volume.set(volume),
                _ => {}
            }
        }
        if let Link::Ended(reason) = receiver.link() {
            bail!("Lost the stream: {reason}");
        }
        let Some(frame) = receiver.next(Duration::from_millis(100)) else {
            continue;
        };
        if decoder
            .as_ref()
            .is_none_or(|decoder| decoder.codec() != frame.codec)
        {
            decoder = Some(Decoder::new(frame.codec)?);
        }
        let Some(decoder) = decoder.as_mut() else {
            continue;
        };
        let picture = match decoder.decode(&frame.data) {
            Ok(Some(picture)) => picture,
            Ok(None) => continue,
            Err(error) => {
                failures += 1;
                // The first of a run, and then now and again.
                if failures == 1 || failures.is_multiple_of(100) {
                    tracing::warn!(
                        error = format!("{error:#}"),
                        failures,
                        keyframe = frame.keyframe,
                        size = frame.data.len(),
                        "a stream frame didn't decode"
                    );
                }
                receiver.request_keyframe();
                continue;
            }
        };
        let (width, height) = (picture.width, picture.height);
        if writer.fits(width, height) && width <= max_width && height <= max_height {
            writer.write(width, height, |pixels| picture.to_rgba(pixels));
        } else {
            let (width, height) = fit(width, height, max_width, max_height);
            writer.write(width, height, |pixels| {
                picture.i420().to_rgba(&mut scratch, pixels, width, height)
            });
        }
        if !shown {
            shown = true;
            tracing::info!(width, height, codec = ?frame.codec, "showing the stream");
            report(Report::Streaming);
        }
    }
    Ok(())
}

/// Sleeps until the next frame is due, or returns false once told to stop.
fn next_frame(started: Instant, frame: u64, fps: u32, stop: &AtomicBool) -> bool {
    let due = started + Duration::from_secs(frame) / fps;
    if let Some(wait) = due.checked_duration_since(Instant::now()) {
        std::thread::sleep(wait);
    }
    !stop.load(Ordering::Relaxed)
}

fn test_pattern(
    mut sink: Sink,
    width: u32,
    height: u32,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<()> {
    let (width, height) = (width.min(640), height.min(360));
    report(Report::Sharing { width, height });
    let started = Instant::now();
    let mut frame = 0u64;
    let mut rgba = vec![0; (width * height * 4) as usize];
    let mut picture = I420Buffer::new(width, height);
    while next_frame(started, frame, sink.fps, stop) {
        let shift = (frame * 4) as u32;
        for (index, pixel) in rgba.chunks_exact_mut(4).enumerate() {
            let (x, y) = (index as u32 % width, index as u32 / width);
            let bar = ((x + shift) * 8 / width) % 8;
            let level = 255 - (y * 128 / height) as u8;
            let on = |bit: u32| if bar & bit != 0 { level } else { 32 };
            pixel.copy_from_slice(&[on(4), on(2), on(1), 255]);
        }
        let (y_stride, u_stride, v_stride) = picture.strides();
        let (y, u, v) = picture.data_mut();
        // libyuv names formats by their order in a little-endian word: its
        // ABGR is R, G, B, A in memory.
        yuv_helper::abgr_to_i420(
            &rgba,
            width * 4,
            y,
            y_stride,
            u,
            u_stride,
            v,
            v_stride,
            width as i32,
            height as i32,
        );
        sink.frame(&mut picture)?;
        // Colour bars are all it shares.
        let _switch = sink.check(report)?;
        frame += 1;
    }
    Ok(())
}

/// The largest frame shared at `resolution`; screens of another shape fit
/// inside it.
fn bounds(resolution: Resolution) -> (u32, u32) {
    match resolution {
        Resolution::P480 => (854, 480),
        Resolution::P720 => (1280, 720),
        Resolution::P1080 => (SHARE_WIDTH, SHARE_HEIGHT),
        Resolution::P1440 => (2560, 1440),
        Resolution::Source => (3840, 2160),
    }
}

/// The size `width` by `height` takes when scaled down to fit `max_width`
/// by `max_height`, keeping its shape. Codecs want even sizes.
pub fn fit(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    let scale = f64::min(
        1.0,
        f64::min(
            f64::from(max_width) / f64::from(width.max(1)),
            f64::from(max_height) / f64::from(height.max(1)),
        ),
    );
    let even = |value: f64| ((value as u32) & !1).max(2);
    (
        even(f64::from(width) * scale),
        even(f64::from(height) * scale),
    )
}

/// Converts an I420 picture into RGBA pixels of its size.
fn to_rgba(picture: &I420Buffer, pixels: &mut [u8]) {
    let (y, u, v) = picture.data();
    let (y_stride, u_stride, v_stride) = picture.strides();
    let (width, height) = (picture.width(), picture.height());
    yuv_helper::i420_to_abgr(
        y,
        y_stride,
        u,
        u_stride,
        v,
        v_stride,
        pixels,
        width * 4,
        width as i32,
        height as i32,
    );
}

// libwebrtc asks the screen sharing portal over D-Bus through GIO, but
// webrtc-sys only compiles against its headers.
#[cfg(target_os = "linux")]
#[link(name = "gio-2.0")]
#[link(name = "gobject-2.0")]
#[link(name = "glib-2.0")]
unsafe extern "C" {
    fn g_main_context_iteration(
        context: *mut std::ffi::c_void,
        may_block: std::ffi::c_int,
    ) -> std::ffi::c_int;
}

/// Runs what's due on GLib's main context. The portal's D-Bus replies arrive
/// there, and nothing else runs it in this process: without this the
/// dialog never opens.
fn run_glib() {
    #[cfg(target_os = "linux")]
    // SAFETY: the default context; only the capture thread iterates it.
    while unsafe { g_main_context_iteration(std::ptr::null_mut(), 0) } != 0 {}
}

#[derive(Default)]
struct Captured {
    size: Option<(u32, u32)>,
    /// Capture stopped for good, or a frame couldn't be sent.
    failed: Option<String>,
}

fn capture_screen(
    source: Source,
    sink: Sink,
    max_width: u32,
    max_height: u32,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<()> {
    let mut fps = sink.fps;
    let (mut max_width, mut max_height) = (max_width, max_height);
    let sink = Arc::new(Mutex::new(sink));
    let mut source = source;
    // A screen on Windows goes through desktop duplication, which gives
    // every frame the screen shows, and so does a window filling its screen
    // while it's in front, as a game being played does: Windows' window
    // capture gives at most about 60 frames a second, and choppy ones at
    // that. Other windows, cameras, and screens it can't duplicate go
    // through WebRTC's capturer. A shared window moves between the two as it
    // comes to the front and leaves it, so the stream never shows what's in
    // front of the window, and a game alt-tabbed away from when sharing
    // started still gets duplication once it's back.
    #[allow(unused_mut)]
    let mut duplicating = cfg!(windows);
    // A window not filling its screen in front goes through Windows' window
    // capture, on the GPU drawing it, until that fails on this system.
    #[allow(unused_mut)]
    let mut window_capturing = cfg!(windows);
    loop {
        #[cfg(windows)]
        if duplicating {
            let window = match source {
                Source::Window(window) => Some(window),
                _ => None,
            };
            let screen = match source {
                Source::Display(id) => Some(Some(id)),
                Source::Screen => Some(None),
                Source::Window(window) => crate::duplication::fronted_screen(window).map(Some),
                _ => None,
            };
            if let Some(id) = screen {
                if window.is_some() {
                    tracing::info!("the window fills its screen in front; sharing it through duplication");
                }
                match capture_duplicated(id, window, &sink, &mut max_width, &mut max_height, stop, report) {
                    Ok(None) => return Ok(()),
                    Ok(Some(next)) => {
                        source = next;
                        continue;
                    }
                    Err(error) if error.downcast_ref::<crate::duplication::Unavailable>().is_some() => {
                        tracing::warn!(error = format!("{error:#}"), "sharing through WebRTC's capturer");
                        duplicating = false;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        #[cfg(windows)]
        if window_capturing && let Source::Window(window) = source {
            match capture_window(window, duplicating, &sink, &mut max_width, &mut max_height, stop, report) {
                Ok(None) => return Ok(()),
                Ok(Some(next)) => {
                    source = next;
                    continue;
                }
                Err(error) if error.downcast_ref::<crate::duplication::Unavailable>().is_some() => {
                    tracing::warn!(error = format!("{error:#}"), "sharing the window through WebRTC's capturer");
                    window_capturing = false;
                }
                Err(error) => return Err(error),
            }
        }
        let limits = (&mut fps, &mut max_width, &mut max_height);
        match capture_captured(source, &sink, limits, duplicating, stop, report)? {
            Some(next) => source = next,
            None => return Ok(()),
        }
    }
}

/// How often a shared window is checked for filling its screen in front,
/// so it moves between duplication and window capture.
#[cfg(windows)]
const FRONT_CHECK: Duration = Duration::from_millis(250);

/// Shares `source` through WebRTC's capturer, or a camera, until stopped.
/// On Windows, gives back a shared window that came to fill its screen in
/// front, to duplicate, when `duplicating` works.
fn capture_captured(
    source: Source,
    sink: &Arc<Mutex<Sink>>,
    (fps, max_width, max_height): (&mut u32, &mut u32, &mut u32),
    duplicating: bool,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<Option<Source>> {
    let _ = duplicating;
    #[cfg(windows)]
    let mut front_checked = Instant::now();
    let sink = sink.clone();
    // What's shared, and what's to be once it gives a picture, so a change of
    // quality starts it again at the new size.
    let mut current = source.clone();
    let mut next_source: Option<Source> = None;
    let mut capture = Capture::start(source, &sink, *max_width, *max_height)?;
    // A screen, window or camera to share in its place, until it gives a
    // picture.
    let mut switching: Option<Capture> = None;
    let mut started = Instant::now();
    let mut frame = 0u64;
    let mut reported = None;
    while next_frame(started, frame, *fps, stop) {
        #[cfg(windows)]
        if duplicating
            && switching.is_none()
            && front_checked.elapsed() >= FRONT_CHECK
            && let Source::Window(window) = current
        {
            front_checked = Instant::now();
            if crate::duplication::fronted_screen(window).is_some() {
                return Ok(Some(current));
            }
        }
        run_glib();
        capture.capture_frame();
        if let Some(next) = &mut switching {
            next.capture_frame();
        }
        frame += 1;
        let seen = capture.seen();
        if let Some(failure) = &seen.failed {
            bail!(if reported.is_none() {
                "Screen sharing was cancelled, or this desktop has no screen sharing dialog."
                    .to_owned()
            } else {
                failure.clone()
            });
        }
        if seen.size != reported
            && let Some((width, height)) = seen.size
        {
            reported = seen.size;
            report(Report::Sharing { width, height });
        }
        drop(seen);
        if let Some(next) = &switching {
            let seen = next.seen();
            if let Some(failure) = &seen.failed {
                tracing::info!(failure, "sharing the same screen or window");
                drop(seen);
                switching = None;
            } else if seen.size.is_some() {
                drop(seen);
                capture = switching.take().expect("switching");
                if let Some(source) = next_source.take() {
                    current = source;
                }
                // Discord shows what's shared now.
                let mut sink = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if sink.streaming {
                    sink.preview_due = Some(Instant::now());
                }
            }
        }
        let (switch, requality) = {
            let mut sink = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let switch = sink.check(report)?;
            (switch, sink.requality.take().map(|size| (size, sink.fps)))
        };
        // A new quality shares the same source again at its size and rate.
        let switch = match requality {
            Some(((width, height), rate)) => {
                (*max_width, *max_height) = (width, height);
                if rate != *fps {
                    (*fps, started, frame) = (rate, Instant::now(), 0);
                }
                switch.or_else(|| Some(next_source.clone().unwrap_or_else(|| current.clone())))
            }
            None => switch,
        };
        if let Some(source) = switch {
            next_source = Some(source.clone());
            // A switch asked for while another waits replaces it.
            switching = None;
            match Capture::start(source, &sink, *max_width, *max_height) {
                Ok(next) => switching = Some(next),
                Err(error) => tracing::warn!(
                    error = format!("{error:#}"),
                    "sharing the same screen or window"
                ),
            }
        }
    }
    Ok(None)
}

/// When nothing on the screen changes, the last frame goes again this
/// often, so a viewer joining or asking for a keyframe gets one.
#[cfg(windows)]
const IDLE_REPEAT: Duration = Duration::from_millis(100);

/// Shares the screen `id` through desktop duplication until stopped, or
/// until another source is asked for, which it returns. Fails with
/// [`crate::duplication::Unavailable`] before sending anything when the
/// screen can't be duplicated.
#[cfg(windows)]
fn capture_duplicated(
    id: Option<u64>,
    window: Option<u64>,
    sink: &Arc<Mutex<Sink>>,
    max_width: &mut u32,
    max_height: &mut u32,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<Option<Source>> {
    let mut duplication = crate::duplication::Duplication::open(id)?;
    // A shared window that left the front, or stopped filling its screen,
    // goes back to window capture before the screen shows what's in front
    // of it.
    let leave = move || {
        let window = window?;
        crate::duplication::fronted_screen(window).is_none().then(|| {
            tracing::info!("the window left the front; sharing it through window capture");
            Source::Window(window)
        })
    };
    capture_frames(&mut duplication, &leave, sink, max_width, max_height, stop, report)
}

/// Shares the window `window` through Windows' window capture until
/// stopped, until another source is asked for, or, when `duplicating`, until
/// it fills its screen in front, when duplication gives more frames: each
/// returned. Fails with [`crate::duplication::Unavailable`] when Windows
/// can't capture it.
#[cfg(windows)]
fn capture_window(
    window: u64,
    duplicating: bool,
    sink: &Arc<Mutex<Sink>>,
    max_width: &mut u32,
    max_height: &mut u32,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<Option<Source>> {
    let mut capture = crate::window_capture::WindowCapture::open(window)?;
    tracing::info!("sharing the window through window capture");
    let leave = move || {
        (duplicating && crate::duplication::fronted_screen(window).is_some())
            .then_some(Source::Window(window))
    };
    capture_frames(&mut capture, &leave, sink, max_width, max_height, stop, report)
}

/// A screen's or window's frames as they're drawn, for [`capture_frames`].
#[cfg(windows)]
trait FrameSource {
    fn next(&mut self, timeout_ms: u32) -> Result<crate::duplication::Next>;
    /// Starts again after [`crate::duplication::Next::Lost`].
    fn reopen(&mut self) -> Result<()>;
    fn has_picture(&self) -> bool;
    /// Calls `show` with the last picture, at its own size.
    fn with_picture(&mut self, show: &mut dyn FnMut(&mut I420Buffer) -> Result<()>) -> Result<()>;
}

#[cfg(windows)]
impl FrameSource for crate::duplication::Duplication {
    fn next(&mut self, timeout_ms: u32) -> Result<crate::duplication::Next> {
        crate::duplication::Duplication::next(self, timeout_ms)
    }
    fn reopen(&mut self) -> Result<()> {
        crate::duplication::Duplication::reopen(self)
    }
    fn has_picture(&self) -> bool {
        crate::duplication::Duplication::has_picture(self)
    }
    fn with_picture(&mut self, show: &mut dyn FnMut(&mut I420Buffer) -> Result<()>) -> Result<()> {
        crate::duplication::Duplication::with_picture(self, show)
    }
}

#[cfg(windows)]
impl FrameSource for crate::window_capture::WindowCapture {
    fn next(&mut self, timeout_ms: u32) -> Result<crate::duplication::Next> {
        crate::window_capture::WindowCapture::next(self, timeout_ms)
    }
    fn reopen(&mut self) -> Result<()> {
        Ok(())
    }
    fn has_picture(&self) -> bool {
        crate::window_capture::WindowCapture::has_picture(self)
    }
    fn with_picture(&mut self, show: &mut dyn FnMut(&mut I420Buffer) -> Result<()>) -> Result<()> {
        show(crate::window_capture::WindowCapture::picture(self))
    }
}

/// How long a shared picture stays black before the log says so: a game
/// whose capture gives black frames, which nothing else would show.
#[cfg(windows)]
const BLACK_WARNING: Duration = Duration::from_secs(5);

/// Sends `frames`' pictures at the stream's rate at most, and the last one
/// again every [`IDLE_REPEAT`] while nothing changes, until stopped, until
/// another source is asked for, or until `leave`, checked every
/// [`FRONT_CHECK`], names where to go instead: each returned.
#[cfg(windows)]
fn capture_frames(
    frames: &mut dyn FrameSource,
    leave: &dyn Fn() -> Option<Source>,
    sink: &Arc<Mutex<Sink>>,
    max_width: &mut u32,
    max_height: &mut u32,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) -> Result<Option<Source>> {
    use crate::duplication::Next;
    let lock = || sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let fps = lock().fps.max(1);
    let mut interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    // A frame up to a quarter early still goes, so jitter in when the screen
    // presents doesn't halve the rate.
    let mut early = interval / 4;
    let mut last_sent: Option<Instant> = None;
    let mut pending = false;
    let mut reported = None;
    let mut front_checked = Instant::now();
    // Since when every picture sent was black, and whether the log said so.
    let mut black_since: Option<Instant> = None;
    let mut said_black = false;
    while !stop.load(Ordering::Relaxed) {
        if front_checked.elapsed() >= FRONT_CHECK {
            front_checked = Instant::now();
            if let Some(next) = leave() {
                return Ok(Some(next));
            }
        }
        let now = Instant::now();
        let wait = match (pending, last_sent) {
            (true, Some(sent)) => (sent + interval).saturating_duration_since(now + early),
            (true, None) => Duration::ZERO,
            (false, Some(sent)) => (sent + IDLE_REPEAT).saturating_duration_since(now),
            (false, None) => IDLE_REPEAT,
        };
        match frames.next(wait.as_millis().min(100) as u32)? {
            Next::Frame => pending = true,
            Next::Timeout => {}
            Next::Lost => {
                // A mode change, or the secure desktop: open it again
                // once it's back.
                std::thread::sleep(Duration::from_millis(50));
                if let Err(error) = frames.reopen() {
                    tracing::debug!(error = format!("{error:#}"), "the screen isn't back yet");
                }
                pending = true;
                continue;
            }
        }
        let now = Instant::now();
        let due = last_sent.map_or(now, |sent| sent + interval);
        let idle = last_sent.is_some_and(|sent| now >= sent + IDLE_REPEAT);
        if frames.has_picture() && ((pending && now + early >= due) || idle) {
            let mut size = (0, 0);
            // The source's own picture, lent: a copy of each frame at the
            // screen's size was 5.5 MiB at 1440p, and its copying.
            frames.with_picture(&mut |picture| {
                if !said_black && pending {
                    if is_black(picture) {
                        let since = *black_since.get_or_insert(now);
                        if now.duration_since(since) >= BLACK_WARNING {
                            said_black = true;
                            tracing::warn!(
                                width = picture.width(),
                                height = picture.height(),
                                "what's shared has been black for 5 seconds"
                            );
                        }
                    } else {
                        black_since = None;
                    }
                }
                let (width, height) = (picture.width(), picture.height());
                size = fit(width, height, *max_width, *max_height);
                let mut scaled = (size != (width, height))
                    .then(|| picture.scale(size.0 as i32, size.1 as i32));
                lock().frame(scaled.as_mut().unwrap_or(picture))
            })?;
            if reported != Some(size) {
                reported = Some(size);
                report(Report::Sharing {
                    width: size.0,
                    height: size.1,
                });
            }
            // Keeps to the rate's schedule, unless it fell a frame behind.
            last_sent = Some(if now.saturating_duration_since(due) < interval {
                due.max(now - early)
            } else {
                now
            });
            pending = false;
        }
        let switch = lock().check(report)?;
        // A new quality takes effect at the next frame.
        let requality = {
            let mut sink = lock();
            sink.requality.take().map(|size| (size, sink.fps.max(1)))
        };
        if let Some(((width, height), rate)) = requality {
            (*max_width, *max_height) = (width, height);
            interval = Duration::from_secs_f64(1.0 / f64::from(rate));
            early = interval / 4;
            pending = true;
        }
        if let Some(next) = switch {
            let mut sink = lock();
            if sink.streaming {
                sink.preview_due = Some(Instant::now());
            }
            return Ok(Some(next));
        }
    }
    Ok(None)
}

/// A capturer sending a screen's, window's or camera's frames to the sink.
struct Capture {
    /// A screen's or window's, which gives a frame when asked.
    capturer: Option<livekit::webrtc::desktop_capturer::DesktopCapturer>,
    /// A camera's, which sends its frames from its own thread.
    _camera: Option<crate::camera::Camera>,
    captured: Arc<Mutex<Captured>>,
}

impl Capture {
    /// Starts capturing `source`, fitting its frames in `max_width` by
    /// `max_height`; on Linux, a screen or window the system's dialog picks.
    fn start(
        source: Source,
        sink: &Arc<Mutex<Sink>>,
        max_width: u32,
        max_height: u32,
    ) -> Result<Self> {
        if let Source::Camera(id) = &source {
            return Self::camera(id, sink, max_width, max_height);
        }
        use livekit::webrtc::desktop_capturer::{
            CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions,
        };
        // On Linux the system's screen sharing dialog picks a screen or window.
        #[cfg(target_os = "linux")]
        let kind = DesktopCaptureSourceType::Generic;
        #[cfg(not(target_os = "linux"))]
        let kind = match source {
            Source::Window(_) => DesktopCaptureSourceType::Window,
            _ => DesktopCaptureSourceType::Screen,
        };
        let mut options = DesktopCapturerOptions::new(kind);
        options.set_include_cursor(true);
        let mut capturer = DesktopCapturer::new(options)
            .context("Screen sharing isn't available on this system.")?;
        let captured = Arc::new(Mutex::new(Captured::default()));
        let (into, seen) = (sink.clone(), captured.clone());
        #[cfg(target_os = "linux")]
        let source = {
            let _ = source;
            None
        };
        #[cfg(not(target_os = "linux"))]
        let source = {
            let mut listed = capturer.get_source_list().into_iter();
            let found = match source {
                Source::Display(id) | Source::Window(id) => listed.find(|listed| listed.id() == id),
                _ => listed.next(),
            };
            Some(found.context("That screen or window is gone.")?)
        };
        capturer.start_capture(source, move |result| {
            let mut seen = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            match result {
                Ok(frame) => {
                    let mut picture = picture_of(&frame);
                    let (width, height) = (picture.width(), picture.height());
                    let size = fit(width, height, max_width, max_height);
                    if size != (width, height) {
                        picture = picture.scale(size.0 as i32, size.1 as i32);
                    }
                    let taken = into
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .frame(&mut picture);
                    match taken {
                        Ok(()) => seen.size = Some(size),
                        Err(error) => seen.failed = Some(format!("{error:#}")),
                    }
                }
                // While the dialog is open, or between frames.
                Err(CaptureError::Temporary) => {}
                Err(CaptureError::Permanent) => {
                    seen.failed
                        .get_or_insert_with(|| "Screen sharing stopped.".into());
                }
            }
        });
        Ok(Self {
            capturer: Some(capturer),
            _camera: None,
            captured,
        })
    }

    /// Films with the camera `id` into the sink, at the stream's size and
    /// frame rate at most. Each picture goes into a buffer kept for the next.
    fn camera(id: &str, sink: &Arc<Mutex<Sink>>, max_width: u32, max_height: u32) -> Result<Self> {
        let fps = sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .fps;
        let captured = Arc::new(Mutex::new(Captured::default()));
        let (into, seen) = (sink.clone(), captured.clone());
        let mut kept: Option<I420Buffer> = None;
        let camera = crate::camera::Camera::open_at(
            Some(id),
            (max_width, max_height, fps),
            Box::new(move |frame| {
                let size = (frame.width, frame.height);
                if kept
                    .as_ref()
                    .is_none_or(|picture| (picture.width(), picture.height()) != size)
                {
                    kept = Some(I420Buffer::new(frame.width, frame.height));
                }
                let picture = kept.as_mut().expect("made above");
                copy_planes(frame, picture);
                // The camera fits the stream already; an odd size, which
                // codecs can't take, loses a row or a column.
                let fitted = fit(size.0, size.1, max_width, max_height);
                let taken = if fitted == size {
                    into.lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .frame(picture)
                } else {
                    let mut scaled = picture.scale(fitted.0 as i32, fitted.1 as i32);
                    into.lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .frame(&mut scaled)
                };
                let mut seen = seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                match taken {
                    Ok(()) => seen.size = Some(fitted),
                    Err(error) => seen.failed = Some(format!("{error:#}")),
                }
            }),
        )?;
        Ok(Self {
            capturer: None,
            _camera: Some(camera),
            captured,
        })
    }

    /// Asks a screen's or window's capturer for a frame; a camera sends its
    /// own.
    fn capture_frame(&mut self) {
        if let Some(capturer) = &mut self.capturer {
            capturer.capture_frame();
        }
    }

    fn seen(&self) -> std::sync::MutexGuard<'_, Captured> {
        self.captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Copies a camera's picture into `picture`, of the same size.
fn copy_planes(frame: &crate::picture::I420<'_>, picture: &mut I420Buffer) {
    let (width, height) = (frame.width as usize, frame.height as usize);
    let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
    let (y_stride, u_stride, v_stride) = picture.strides();
    let into_strides = [y_stride, u_stride, v_stride];
    let (y, u, v) = picture.data_mut();
    for (plane, (from, into)) in [(frame.y, y), (frame.u, u), (frame.v, v)]
        .into_iter()
        .enumerate()
    {
        let (width, height) = if plane == 0 {
            (width, height)
        } else {
            (chroma_width, chroma_height)
        };
        let rows = from
            .chunks(frame.strides[plane] as usize)
            .zip(into.chunks_mut(into_strides[plane] as usize));
        for (from, into) in rows.take(height) {
            into[..width].copy_from_slice(&from[..width]);
        }
    }
}

/// A captured frame as an I420 picture of its size.
fn picture_of(frame: &livekit::webrtc::desktop_capturer::DesktopFrame) -> I420Buffer {
    let (width, height) = (frame.width().max(1) as u32, frame.height().max(1) as u32);
    let mut picture = I420Buffer::new(width, height);
    let (y_stride, u_stride, v_stride) = picture.strides();
    let (y, u, v) = picture.data_mut();
    // libyuv's ARGB is B, G, R, A in memory, as the capturer's frames are.
    yuv_helper::argb_to_i420(
        frame.data(),
        frame.stride(),
        y,
        y_stride,
        u,
        u_stride,
        v,
        v_stride,
        width as i32,
        height as i32,
    );
    picture
}

/// How long [`glance`] waits for a picture of a screen that isn't black,
/// before showing the last it took, or none.
#[cfg(not(target_os = "linux"))]
const GLANCE_WAIT: Duration = Duration::from_millis(1500);

/// Takes a picture of each of `screens`, as [`Job::Glance`] asks. A screen
/// that gives none goes without, and the rest still come.
fn glance(
    screens: &[u64],
    window: bool,
    max_width: u32,
    max_height: u32,
    stop: &AtomicBool,
    report: &mut dyn FnMut(Report),
) {
    // On Linux the system's dialog shows what there is to share.
    #[cfg(target_os = "linux")]
    let _ = (screens, window, max_width, max_height, stop, report);
    #[cfg(not(target_os = "linux"))]
    for &id in screens {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match glance_at(id, window, stop) {
            Ok(Some(mut picture)) => {
                let (width, height) = fit(picture.width(), picture.height(), max_width, max_height);
                if (width, height) != (picture.width(), picture.height()) {
                    picture = picture.scale(width as i32, height as i32);
                }
                let mut rgba = vec![0; (width * height * 4) as usize];
                to_rgba(&picture, &mut rgba);
                report(Report::Screen {
                    id,
                    window,
                    width,
                    height,
                    rgba,
                });
            }
            Ok(None) => tracing::info!(id, "no picture of the screen came"),
            Err(error) => tracing::warn!(id, "no picture of the screen: {error:#}"),
        }
    }
}

/// A picture of the screen, or the window, listed by `id`, at its own size.
#[cfg(not(target_os = "linux"))]
fn glance_at(id: u64, window: bool, stop: &AtomicBool) -> Result<Option<I420Buffer>> {
    use livekit::webrtc::desktop_capturer::{
        CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions,
    };
    let mut capturer = DesktopCapturer::new(DesktopCapturerOptions::new(if window {
        DesktopCaptureSourceType::Window
    } else {
        DesktopCaptureSourceType::Screen
    }))
    .context("Screen sharing isn't available on this system.")?;
    let source = capturer
        .get_source_list()
        .into_iter()
        .find(|listed| listed.id() == id)
        .context("That screen is gone.")?;
    // The last picture taken, and whether capture stopped for good.
    let taken = Arc::new(Mutex::new((None::<I420Buffer>, false)));
    let into = taken.clone();
    capturer.start_capture(Some(source), move |result| {
        let mut taken = into.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match result {
            Ok(frame) => taken.0 = Some(picture_of(&frame)),
            Err(CaptureError::Temporary) => {}
            Err(CaptureError::Permanent) => taken.1 = true,
        }
    });
    // A screen's first picture can come black, before the system has
    // drawn it for the capturer.
    // A window has drawn already, unless minimized, which gives nothing.
    let deadline = Instant::now() + if window { GLANCE_WAIT / 3 } else { GLANCE_WAIT };
    loop {
        capturer.capture_frame();
        let mut taken = taken
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let lit = taken.0.as_ref().is_some_and(|picture| !is_black(picture));
        if lit || taken.1 || Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
            return Ok(taken.0.take());
        }
        drop(taken);
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Pixels in a report as base64: a third bigger than they are, where JSON's
/// array of numbers would be up to four times.
mod base64_pixels {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(pixels: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(pixels))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        // base64 has nothing JSON escapes, so the line lends it as it is.
        let text = <&str>::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}

/// Whether streams encode on the GPU where it has an encoder: Settings'
/// choice, on until the UI says.
static HARDWARE: AtomicBool = AtomicBool::new(true);

/// Encodes streams started from now on with the GPU's encoder, or not.
pub fn set_hardware_encoding(on: bool) {
    HARDWARE.store(on, Ordering::Relaxed);
}

/// Discord Obsidian's handle on a running worker. Dropping it stops the worker.
pub struct Worker {
    child: Child,
    stdin: Option<ChildStdin>,
}

impl Worker {
    /// Starts sharing `source`, with frames for the UI in a new [`Frames`].
    pub fn share(
        source: Source,
        quality: StreamQuality,
    ) -> Result<(Self, Frames, mpsc::UnboundedReceiver<Report>)> {
        let source = picked(source);
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT)?;
        let (width, height) = bounds(quality.resolution);
        let job = Job::Share {
            source,
            frames: frames.path().context("the video frames file has no name")?,
            width,
            height,
            fps: quality.fps,
            effort: if quality.high_quality {
                Effort::High
            } else {
                Effort::Light
            },
            hardware: HARDWARE.load(Ordering::Relaxed),
            sound: quality.audio,
        };
        let (worker, reports) = Self::start(&job)?;
        Ok((worker, frames, reports))
    }

    /// Starts taking pictures of `screens` for the share menu, each fitting
    /// `width` by `height`. They come as [`Report::Screen`]s, then
    /// [`Report::Ended`].
    pub fn glance(
        screens: Vec<u64>,
        windows: Vec<u64>,
        width: u32,
        height: u32,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Report>)> {
        Self::start(&Job::Glance {
            screens,
            windows,
            width,
            height,
        })
    }

    /// Starts watching the stream at `server`, with pictures for the UI in a
    /// new [`Frames`].
    pub fn watch(
        server: &StreamServer,
        speakers: Option<String>,
        deafened: bool,
        volume: f32,
    ) -> Result<(Self, Frames, mpsc::UnboundedReceiver<Report>)> {
        let frames = Frames::create(WATCH_WIDTH, WATCH_HEIGHT)?;
        let job = Job::Watch {
            server: server.clone(),
            frames: frames.path().context("the video frames file has no name")?,
            width: WATCH_WIDTH,
            height: WATCH_HEIGHT,
            speakers,
            deafened,
            volume,
        };
        let (worker, reports) = Self::start(&job)?;
        Ok((worker, frames, reports))
    }

    /// Starts playing the video file at `file`, which may still be
    /// downloading, with pictures that fit `width` by `height` for the UI in
    /// a new [`Frames`].
    pub fn play(
        file: PathBuf,
        size: Option<u64>,
        (width, height): (u32, u32),
        speakers: Option<String>,
        volume: f32,
    ) -> Result<(Self, Frames, mpsc::UnboundedReceiver<Report>)> {
        let frames = Frames::create(WATCH_WIDTH, WATCH_HEIGHT)?;
        let job = Job::Play {
            file,
            size,
            frames: frames.path().context("the video frames file has no name")?,
            width: width.min(WATCH_WIDTH),
            height: height.min(WATCH_HEIGHT),
            speakers,
            volume,
        };
        let (worker, reports) = Self::start(&job)?;
        Ok((worker, frames, reports))
    }

    /// Tells a playing video worker what to do.
    pub fn tell(&mut self, control: &Control) -> Result<()> {
        self.control(control)
    }

    /// Tells a watching worker whether to play the stream's sound.
    pub fn hear(&mut self, hearing: bool) -> Result<()> {
        self.control(&Control::Hear(hearing))
    }

    /// Tells a watching worker how loud to play the stream's sound.
    pub fn volume(&mut self, volume: f32) -> Result<()> {
        self.control(&Control::Volume(volume))
    }

    /// Hands the worker the server Discord started the stream on.
    pub fn send(&mut self, server: &StreamServer) -> Result<()> {
        self.control(&Control::Send(server.clone()))
    }

    /// Tells a sharing worker to share at `quality` from now on.
    pub fn requality(&mut self, quality: StreamQuality) -> Result<()> {
        let (width, height) = bounds(quality.resolution);
        self.control(&Control::Quality {
            width,
            height,
            fps: quality.fps,
        })?;
        self.control(&Control::Sound(quality.audio))
    }

    /// Tells a sharing worker to share `source` in place of what it shares.
    pub fn switch(&mut self, source: Source) -> Result<()> {
        self.control(&Control::Switch(picked(source)))
    }

    fn control(&mut self, control: &Control) -> Result<()> {
        let line = serde_json::to_string(control)?;
        let stdin = self
            .stdin
            .as_mut()
            .context("the stream worker has stopped")?;
        writeln!(stdin, "{line}").context("could not reach the stream worker")?;
        Ok(())
    }

    fn start(job: &Job) -> Result<(Self, mpsc::UnboundedReceiver<Report>)> {
        // Tests run from their own binary, so they name Discord Obsidian's.
        let program = match std::env::var_os("OBSIDIAN_STREAM_WORKER") {
            Some(program) => PathBuf::from(program),
            None => running_obsidian()?,
        };
        let mut child = Command::new(program)
            .arg(ARGUMENT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(crate::logging::for_worker())
            .spawn()
            .context("could not start the stream worker")?;
        let mut stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let line = serde_json::to_string(job)?;
        if let Some(stdin) = stdin.as_mut() {
            writeln!(stdin, "{line}").context("could not reach the stream worker")?;
        }
        let (reports, receiver) = mpsc::unbounded_channel();
        let unexpected = match job {
            Job::Play { .. } => "The video stopped unexpectedly.",
            _ => "Screen sharing stopped unexpectedly.",
        };
        std::thread::Builder::new()
            .name("obsidian-stream-reports".into())
            .spawn(move || {
                let mut ended = false;
                for line in stdout
                    .into_iter()
                    .flat_map(|stdout| BufReader::new(stdout).lines().map_while(Result::ok))
                {
                    match serde_json::from_str::<Report>(&line) {
                        Ok(report) => {
                            ended |= matches!(report, Report::Ended { .. });
                            if reports.send(report).is_err() {
                                return;
                            }
                        }
                        Err(error) => tracing::warn!(%error, "unknown stream worker report"),
                    }
                }
                if !ended {
                    let _ = reports.send(Report::Ended {
                        error: Some(unexpected.into()),
                    });
                }
            })
            .context("could not start the stream worker")?;
        Ok((Self { child, stdin }, receiver))
    }
}

/// The Discord Obsidian that's running, to start as the worker. On Linux that's
/// `/proc/self/exe`, which still opens after a rebuild or an update replaced
/// Discord Obsidian's file, where its path names a deleted file; it also keeps the worker
/// the same version as the window it talks to.
fn running_obsidian() -> Result<PathBuf> {
    if cfg!(target_os = "linux") {
        return Ok(PathBuf::from("/proc/self/exe"));
    }
    std::env::current_exe().context("could not find Discord Obsidian to start streaming")
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = writeln!(stdin, "stop");
        }
        // The worker checks for the stop between frames.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        tracing::warn!("the stream worker didn't stop; ending it");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_is_the_running_obsidian() {
        // Here that's the test binary, which lists its tests and exits.
        let program = running_obsidian().unwrap();
        let status = std::process::Command::new(program)
            .args(["--list", "--format", "terse"])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn fits_frames_with_even_sizes() {
        assert_eq!(fit(1920, 1080, 1280, 720), (1280, 720));
        assert_eq!(fit(3440, 1440, 1280, 720), (1280, 534));
        assert_eq!(fit(801, 601, 1280, 720), (800, 600), "never larger");
    }

    #[test]
    fn scales_a_frame_a_few_pixels_off_instead_of_opening_again() {
        assert!(near((1568, 1078), (1568, 1080)), "a window's row");
        assert!(near((1920, 1080), (1920, 1040)), "a taskbar's height");
        assert!(!near((1280, 720), (1920, 1080)), "another quality");
        assert!(!near((1080, 1920), (1920, 1080)), "another shape");
    }

    #[test]
    fn aims_for_discords_bitrates() {
        assert_eq!(kbps(720, 30, None), 3500);
        assert_eq!(kbps(1080, 60, None), 9000);
        assert_eq!(kbps(1080, 240, None), 36000, "more for more frames");
        assert_eq!(kbps(1440, 360, None), 40000, "up to 40 Mbit/s");
        assert_eq!(kbps(1440, 60, None), 20000, "1440p keeps its detail");
        assert_eq!(kbps(1080, 60, Some(4_000_000)), 4000, "lowered for loss");
        assert_eq!(kbps(720, 30, Some(9_000_000)), 3500, "never past the cap");
        assert_eq!(kbps(1080, 60, Some(100_000)), 500, "nor below a screen's floor");
    }

    /// With the GPU, AV1 and H265 before H264 where they open, and an
    /// encoder for the first; without it, H264 alone, on x264.
    #[test]
    fn offers_the_gpus_codecs_first() {
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let (_controls, received) = channel::channel();
        let mut sink = Sink {
            writer: Writer::open(&frames.path().unwrap()).unwrap(),
            controls: received,
            server: None,
            sound: false,
            video: None,
            fps: 30,
            effort: Effort::Light,
            hardware: true,
            started: Instant::now(),
            streaming: false,
            preview_due: None,
            thumbnail: None,
            lost: None,
            previewed: None,
            requality: None,
            reopen: false,
        };
        let (offers, encoder) = sink.offers(320, 180).unwrap();
        eprintln!("offering {offers:?}, encoding with {}", encoder.engine());
        assert_eq!(offers.last(), Some(&Codec::H264));
        let mut order = [Codec::Av1, Codec::H265, Codec::H264].iter();
        assert!(
            offers.iter().all(|codec| order.any(|next| next == codec)),
            "in order"
        );
        assert_eq!(encoder.codec(), offers[0]);
        sink.hardware = false;
        let (offers, encoder) = sink.offers(320, 180).unwrap();
        assert_eq!((offers, encoder.engine()), (vec![Codec::H264], "x264"));
    }

    #[test]
    fn previews_what_it_sends_at_a_smaller_size() {
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let (_controls, received) = channel::channel();
        let mut sink = Sink {
            writer: Writer::open(&frames.path().unwrap()).unwrap(),
            controls: received,
            server: None,
            sound: false,
            video: None,
            fps: 30,
            effort: Effort::Light,
            hardware: false,
            started: Instant::now(),
            streaming: false,
            preview_due: None,
            thumbnail: None,
            lost: None,
            previewed: None,
            requality: None,
            reopen: false,
        };
        // A 1080p picture, mid-grey: luma and chroma halfway.
        let mut picture = I420Buffer::new(SHARE_WIDTH, SHARE_HEIGHT);
        let (y, u, v) = picture.data_mut();
        y.fill(126);
        u.fill(128);
        v.fill(128);
        sink.frame(&mut picture).unwrap();
        let (width, height, first) = frames
            .take(|width, height, pixels| (width, height, pixels[..4].to_vec()))
            .unwrap();
        assert_eq!((width, height), (PREVIEW_WIDTH, PREVIEW_HEIGHT));
        assert!(
            first[..3].iter().all(|&level| level.abs_diff(128) < 4),
            "{first:?}"
        );
        assert_eq!(first[3], 255);
    }

    #[test]
    fn switches_to_the_last_source_asked_for() {
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let (controls, received) = channel::channel();
        let mut sink = Sink {
            writer: Writer::open(&frames.path().unwrap()).unwrap(),
            controls: received,
            server: None,
            sound: false,
            video: None,
            fps: 30,
            effort: Effort::Light,
            hardware: false,
            started: Instant::now(),
            streaming: false,
            preview_due: None,
            thumbnail: None,
            lost: None,
            previewed: None,
            requality: None,
            reopen: false,
        };
        let mut report = |_| {};
        assert_eq!(sink.check(&mut report).unwrap(), None);
        controls.send(Control::Switch(Source::Display(1))).unwrap();
        controls.send(Control::Switch(Source::Window(2))).unwrap();
        assert_eq!(sink.check(&mut report).unwrap(), Some(Source::Window(2)));
        assert_eq!(sink.check(&mut report).unwrap(), None);
    }

    #[test]
    fn previews_a_stream_once_its_picture_shows() {
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let (_controls, received) = channel::channel();
        let mut sink = Sink {
            writer: Writer::open(&frames.path().unwrap()).unwrap(),
            controls: received,
            server: None,
            sound: false,
            video: None,
            fps: 30,
            effort: Effort::Light,
            hardware: false,
            started: Instant::now(),
            streaming: false,
            preview_due: Some(Instant::now()),
            thumbnail: None,
            lost: None,
            previewed: None,
            requality: None,
            reopen: false,
        };
        let mut picture = I420Buffer::new(SHARE_WIDTH, SHARE_HEIGHT);
        let (y, u, v) = picture.data_mut();
        y.fill(16);
        u.fill(128);
        v.fill(128);
        sink.frame(&mut picture).unwrap();
        assert_eq!(sink.thumbnail, None, "not while the screen is black");
        picture.data_mut().0.fill(126);
        sink.frame(&mut picture).unwrap();
        let thumbnail = sink.thumbnail.take().expect("a preview");
        let jpeg = thumbnail
            .strip_prefix("data:image/jpeg;base64,")
            .map(|data| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .unwrap()
            })
            .unwrap();
        let shown = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((shown.width(), shown.height()), (512, 288));
        sink.frame(&mut picture).unwrap();
        assert_eq!(sink.thumbnail, None, "the next one comes minutes later");
    }

    #[test]
    fn serves_a_test_pattern_until_stopped() {
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let job = Job::Share {
            source: Source::TestPattern,
            frames: frames.path().unwrap(),
            width: PREVIEW_WIDTH,
            height: PREVIEW_HEIGHT,
            fps: 60,
            effort: Effort::Light,
            hardware: false,
            sound: true,
        };
        let (input, mut feed) = std::io::pipe().unwrap();
        writeln!(feed, "{}", serde_json::to_string(&job).unwrap()).unwrap();
        let (sender, reports) = std::sync::mpsc::channel();
        let serving = std::thread::spawn(move || {
            serve(Box::new(BufReader::new(input)), &mut |report| {
                sender.send(report).unwrap();
            });
        });
        assert_eq!(reports.recv().unwrap(), Report::Ready);
        assert_eq!(
            reports.recv().unwrap(),
            Report::Sharing {
                width: 640,
                height: 360
            }
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let size = loop {
            if let Some(size) = frames.take(|width, height, pixels| (width, height, pixels.len())) {
                break size;
            }
            assert!(Instant::now() < deadline, "no frame arrived");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(size, (640, 360, 640 * 360 * 4));
        writeln!(feed, "stop").unwrap();
        serving.join().unwrap();
        assert_eq!(reports.recv().unwrap(), Report::Ended { error: None });
    }

    #[test]
    fn shares_a_camera_until_stopped() {
        // SAFETY: tests that read this variable all set it the same way.
        unsafe { std::env::set_var("OBSIDIAN_CAMERA_TEST_PATTERN", "1") };
        let listed = shareable();
        let camera = listed
            .iter()
            .find(|listed| listed.title == "Test pattern")
            .expect("the test pattern camera is listed");
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let job = Job::Share {
            source: camera.source.clone(),
            frames: frames.path().unwrap(),
            width: PREVIEW_WIDTH,
            height: PREVIEW_HEIGHT,
            fps: 30,
            effort: Effort::Light,
            hardware: false,
            sound: true,
        };
        let (input, mut feed) = std::io::pipe().unwrap();
        writeln!(feed, "{}", serde_json::to_string(&job).unwrap()).unwrap();
        let (sender, reports) = std::sync::mpsc::channel();
        let serving = std::thread::spawn(move || {
            serve(Box::new(BufReader::new(input)), &mut |report| {
                sender.send(report).unwrap();
            });
        });
        assert_eq!(reports.recv().unwrap(), Report::Ready);
        assert_eq!(
            reports.recv_timeout(Duration::from_secs(5)).unwrap(),
            Report::Sharing {
                width: 640,
                height: 360
            }
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let size = loop {
            if let Some(size) = frames.take(|width, height, pixels| (width, height, pixels.len())) {
                break size;
            }
            assert!(Instant::now() < deadline, "no frame arrived");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(size, (640, 360, 640 * 360 * 4));
        writeln!(feed, "stop").unwrap();
        serving.join().unwrap();
        assert_eq!(reports.recv().unwrap(), Report::Ended { error: None });
    }

    #[test]
    fn a_screen_picture_survives_its_report_line() {
        let report = Report::Screen {
            id: 7,
            window: false,
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let line = serde_json::to_string(&report).unwrap();
        assert!(line.contains(r#""rgba":"/wAA/wAA//8=""#), "{line}");
        assert_eq!(serde_json::from_str::<Report>(&line).unwrap(), report);
    }

    /// Takes a picture of each listed screen, as the share menu shows them.
    #[test]
    #[ignore = "needs a desktop where Discord Obsidian lists what to share"]
    fn glances_at_the_listed_screens() {
        let screens: Vec<u64> = shareable()
            .into_iter()
            .filter_map(|listed| match listed.source {
                Source::Display(id) => Some(id),
                _ => None,
            })
            .collect();
        assert!(!screens.is_empty(), "no screen");
        let windows: Vec<u64> = shareable()
            .into_iter()
            .filter_map(|listed| match listed.source {
                Source::Window(id) => Some(id),
                _ => None,
            })
            .collect();
        let job = Job::Glance {
            screens: screens.clone(),
            windows: windows.clone(),
            width: 320,
            height: 180,
        };
        let (input, mut feed) = std::io::pipe().unwrap();
        writeln!(feed, "{}", serde_json::to_string(&job).unwrap()).unwrap();
        let mut reports = vec![];
        serve(Box::new(BufReader::new(input)), &mut |report| {
            reports.push(report)
        });
        let pictured: Vec<u64> = reports
            .iter()
            .filter_map(|report| match report {
                Report::Screen {
                    id,
                    window,
                    width,
                    height,
                    rgba,
                } => {
                    eprintln!("{} {id}: {width}x{height}", if *window { "window" } else { "screen" });
                    if *window {
                        return None;
                    }
                    assert!(*width <= 320 && *height <= 180);
                    assert_eq!(rgba.len(), (width * height * 4) as usize);
                    Some(*id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(pictured, screens);
        assert_eq!(reports.last(), Some(&Report::Ended { error: None }));
    }

    /// Lists what there is to share, and shares the first screen and the
    /// first window for a moment each.
    /// Shares the listed window whose title holds OBSIDIAN_SHARE_WINDOW for
    /// two seconds and says how much of its picture isn't black, as a game
    /// that captures black would show: `OBSIDIAN_SHARE_WINDOW=VALORANT cargo
    /// test -p obsidian-core shares_a_named_window -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a desktop and OBSIDIAN_SHARE_WINDOW"]
    fn shares_a_named_window() {
        let wanted = std::env::var("OBSIDIAN_SHARE_WINDOW").expect("OBSIDIAN_SHARE_WINDOW");
        let listed = shareable();
        let found = listed
            .iter()
            .find(|listed| matches!(listed.source, Source::Window(_)) && listed.title.contains(&wanted))
            .unwrap_or_else(|| panic!("no window titled {wanted}"));
        let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
        let job = Job::Share {
            source: found.source.clone(),
            frames: frames.path().unwrap(),
            width: PREVIEW_WIDTH,
            height: PREVIEW_HEIGHT,
            fps: 60,
            effort: Effort::Light,
            hardware: false,
            sound: false,
        };
        let (input, mut feed) = std::io::pipe().unwrap();
        writeln!(feed, "{}", serde_json::to_string(&job).unwrap()).unwrap();
        let (sender, reports) = std::sync::mpsc::channel();
        let serving = std::thread::spawn(move || {
            serve(Box::new(BufReader::new(input)), &mut |report| {
                let _ = sender.send(report);
            });
        });
        assert_eq!(reports.recv().unwrap(), Report::Ready);
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut last, mut pictures) = (None, 0);
        while Instant::now() < deadline {
            let taken = frames.take(|width, height, pixels| {
                let lit = pixels.chunks_exact(4).filter(|pixel| pixel[..3].iter().any(|&c| c > 24));
                (width, height, lit.count() as f32 / (pixels.len() / 4).max(1) as f32)
            });
            pictures += usize::from(taken.is_some());
            last = taken.or(last);
            std::thread::sleep(Duration::from_millis(5));
        }
        writeln!(feed, "stop").unwrap();
        serving.join().unwrap();
        let (width, height, lit) = last.expect("no frame arrived");
        eprintln!(
            "{:?}: {width}x{height}, {pictures} previews, {:.0}% of it isn't black",
            found.title,
            lit * 100.0
        );
    }

    #[test]
    #[ignore = "needs a desktop where Discord Obsidian lists what to share"]
    fn shares_a_listed_screen_and_window() {
        let listed = shareable();
        eprintln!("{listed:#?}");
        let first = |window: bool| {
            listed
                .iter()
                .find(|listed| matches!(listed.source, Source::Window(_)) == window)
                .map(|listed| listed.source.clone())
        };
        let sources = [
            first(false).expect("no screen"),
            first(true).expect("no window"),
        ];
        for source in sources {
            let frames = Frames::create(PREVIEW_WIDTH, PREVIEW_HEIGHT).unwrap();
            let job = Job::Share {
                source: source.clone(),
                frames: frames.path().unwrap(),
                width: PREVIEW_WIDTH,
                height: PREVIEW_HEIGHT,
                fps: 30,
                effort: Effort::Light,
                hardware: false,
                sound: true,
            };
            let (input, mut feed) = std::io::pipe().unwrap();
            writeln!(feed, "{}", serde_json::to_string(&job).unwrap()).unwrap();
            let (sender, reports) = std::sync::mpsc::channel();
            let serving = std::thread::spawn(move || {
                serve(Box::new(BufReader::new(input)), &mut |report| {
                    sender.send(report).unwrap();
                });
            });
            assert_eq!(reports.recv().unwrap(), Report::Ready);
            let sharing = reports.recv_timeout(Duration::from_secs(10)).unwrap();
            eprintln!("{source:?}: {sharing:?}");
            assert!(matches!(sharing, Report::Sharing { .. }), "{sharing:?}");
            // How much of the last picture isn't black, a second and a half
            // in: what's shared may still be drawing at first.
            let deadline = Instant::now() + Duration::from_millis(1500);
            let mut last = None;
            while Instant::now() < deadline {
                let taken = frames.take(|width, height, pixels| {
                    let lit = pixels.chunks_exact(4).filter(|pixel| pixel[..3] != [0; 3]);
                    (
                        width,
                        height,
                        lit.count() as f32 / (pixels.len() / 4).max(1) as f32,
                    )
                });
                last = taken.or(last);
                std::thread::sleep(Duration::from_millis(5));
            }
            let (width, height, lit) = last.expect("no frame arrived");
            eprintln!(
                "{source:?}: {width}x{height}, {:.0}% of it isn't black",
                lit * 100.0
            );
            writeln!(feed, "stop").unwrap();
            serving.join().unwrap();
            // The size may change as capture settles.
            let ended = reports
                .iter()
                .find(|report| !matches!(report, Report::Sharing { .. }));
            assert_eq!(ended, Some(Report::Ended { error: None }));
        }
        // A window closed since it was listed.
        let frames = Frames::create(16, 16).unwrap();
        let gone = Job::Share {
            source: Source::Window(u64::MAX),
            frames: frames.path().unwrap(),
            width: 16,
            height: 16,
            fps: 1,
            effort: Effort::Light,
            hardware: false,
            sound: true,
        };
        let mut reports = Vec::new();
        let input = format!("{}\n", serde_json::to_string(&gone).unwrap());
        serve(Box::new(std::io::Cursor::new(input)), &mut |report| {
            reports.push(report);
        });
        assert_eq!(
            reports,
            [
                Report::Ready,
                Report::Ended {
                    error: Some("That screen or window is gone.".into())
                }
            ]
        );
    }
}
