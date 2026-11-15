//! A Go Live stream's media connection, to the server Discord assigned the
//! stream: sending this account's stream, or receiving someone else's. It's
//! a voice connection of its own (`voice_connection.rs`), identifying with
//! the stream's RTC server and video. Discord Obsidian sends H264 and receives VP8 or
//! H264.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc as channel,
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

use crate::{
    congestion::{self, DelayBased, SendHistory},
    rtp::{self, Assembler, Codec, Packetizer},
    stream::{self, StreamServer},
    voice_connection::{Connection, Identity, lock, receive, send_json},
};

/// How often a viewer may ask for a keyframe while frames are lost.
const KEYFRAME_REQUESTS: Duration = Duration::from_millis(500);
/// How often a streamer sends an RTCP sender report.
const SENDER_REPORTS: Duration = Duration::from_secs(1);
/// How often a viewer logs what it received while no picture comes through.
const LOG_EVERY: Duration = Duration::from_secs(5);
/// The most packets resent for one NACK, which can name hundreds.
const RESENDS: usize = 128;
/// The most video SSRCs a watched stream puts frames together for.
const MAX_SSRCS: usize = 4;
/// While nothing waits to be sent, a frame's first `BURST` packets go at
/// once; everything else goes at [`pace`]'s rate: a 1080p keyframe sent at
/// once can overflow buffers on the way.
const BURST: usize = 8;
/// The slowest packets are paced.
const PACE_FLOOR: u64 = 1_000_000;

/// The bit/s packets are paced at for a stream aiming at `target` bit/s:
/// a quarter more, as Discord's client sends. Captured from it, a 20 KB
/// keyframe of a 1.5 Mbit/s stream went out over 86 ms, at 1.9 Mbit/s,
/// and no tenth of a second carried more than 2 Mbit/s. The largest frame
/// the encoders' eighth of a second buffer allows leaves in 100 ms.
fn pace(target: u64) -> u64 {
    (target * 5 / 4).max(PACE_FLOOR)
}

/// Keyframes no closer than this, however many viewers ask: each is the
/// largest frame there is. Discord's client, captured answering viewers'
/// picture loss indications, sent them half a second apart at the closest.
pub const KEYFRAME_GAP: Duration = Duration::from_millis(500);

/// How long `bytes` take at `rate` bit/s.
fn airtime(bytes: usize, rate: u64) -> Duration {
    Duration::from_nanos(bytes as u64 * 8 * 1_000_000_000 / rate.max(1))
}

/// The fewest bit/s the rate control lowers a stream to: Discord's client's
/// floor for a screen.
pub const MIN_BITRATE: u64 = 500_000;

/// Where a stream's bitrate starts, about Discord's server's first estimate
/// (4.2 Mbit/s): starting at a 1080p360 stream's 36 Mbit/s cap lost three
/// packets in four on a home connection, and the resends that followed
/// overflowed Windows' buffers.
pub const START_BITRATE: u64 = 4_000_000;

/// The share of the target bitrate packets sent again may take: under
/// heavy loss, resending everything viewers ask for doubles what's sent.
const RESEND_SHARE: u64 = 4;

/// How a stream's bitrate follows the packets lost on the way, as
/// libwebrtc's loss-based estimate does in Discord's client: it starts at
/// [`START_BITRATE`], grows 8% a second while under 2% of packets are lost,
/// up to the cap for the stream's quality, holds between 2% and 10% and
/// while nothing says, and above that drops by half the fraction lost.
///
/// It grows only while the encoder uses what it has: never past one and a
/// half times what the server acknowledged receiving, as libwebrtc limits
/// an application-limited sender. A calm screen sends far below its target,
/// and growing it anyway let the next busy scene burst far past what the
/// path carried: streams ran to their cap while 9 Mbit/s got through, then
/// lost 30% to 60% of their packets, every 30 to 60 seconds.
///
/// Discord's server also sends an estimate (REMB), but it only grows past
/// what it receives once the sender probes with padding, as libwebrtc does.
/// Without probing, following it held 1080p streams at 1.5 Mbit/s on a
/// path that carried far more, so Discord Obsidian goes by loss alone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateControl {
    bitrate: u64,
    /// The bitrate heavy loss last came at. The bitrate stays below
    /// [`CEILING_HOLD`] of it, and it rises a tenth after each
    /// [`RAISE_CEILING`] seconds without loss.
    ceiling: Option<u64>,
    /// Seconds in a row with hardly any loss.
    clear: u32,
}

/// Seconds without loss after which the ceiling heavy loss left rises a
/// tenth, so a path that got better is found again, a step at a time.
const RAISE_CEILING: u32 = 90;
/// The share of the ceiling the bitrate stays under. Creeping on at 1% a
/// second past where loss came ran into the same loss 20 to 40 seconds
/// later, each time freezing viewers.
const CEILING_HOLD: f64 = 0.9;

impl RateControl {
    pub fn new(cap: u64) -> Self {
        Self {
            bitrate: START_BITRATE.min(cap),
            ceiling: None,
            clear: 0,
        }
    }

    pub fn bitrate(&self) -> u64 {
        self.bitrate
    }

    /// Takes a second's `loss`, a fraction of the packets sent, and the
    /// bit/s the server acknowledged lately, and gives the bit/s to aim for
    /// from now on, at most `cap`. Within 15% of where heavy loss last came
    /// it grows 1% a second instead of 8%, and not past [`CEILING_HOLD`]
    /// of it.
    pub fn update(&mut self, loss: Option<f64>, cap: u64, acked: Option<f64>) -> u64 {
        let rate = self.bitrate as f64;
        let next = match loss {
            Some(loss) if loss > 0.10 => {
                self.ceiling = Some(self.bitrate);
                self.clear = 0;
                rate * (1.0 - 0.5 * loss.min(1.0))
            }
            Some(loss) if loss >= 0.02 => {
                self.clear = 0;
                rate
            }
            Some(_) => {
                self.clear += 1;
                if self.clear >= RAISE_CEILING {
                    self.clear = 0;
                    self.ceiling = self
                        .ceiling
                        .map(|ceiling| ceiling + ceiling / 10)
                        .filter(|&ceiling| ceiling < cap);
                }
                let near = self
                    .ceiling
                    .is_some_and(|ceiling| rate >= ceiling as f64 * 0.85);
                let mut next = if near { rate * 1.01 + 1000.0 } else { rate * 1.08 + 1000.0 };
                if let Some(ceiling) = self.ceiling {
                    next = next.min((ceiling as f64 * CEILING_HOLD).max(rate));
                }
                if let Some(acked) = acked {
                    next = next.min((1.5 * acked + 10_000.0).max(rate));
                }
                next
            }
            // Too few packets to tell.
            None => rate,
        };
        self.bitrate = (next.round() as u64).clamp(MIN_BITRATE.min(cap), cap);
        self.bitrate
    }

    /// Keeps the bitrate within a new `cap`, as when the size or rate changes.
    pub fn cap(&mut self, cap: u64) {
        self.bitrate = self.bitrate.min(cap);
    }
}

/// The packets lost on the way in a second: from the server's receiver
/// reports on the video it got from Discord Obsidian, and from viewers'
/// NACKs, each lost packet once however many asked for it.
#[derive(Default)]
struct Loss {
    /// The server's last report: cumulative packets lost and the highest
    /// sequence number received, extended.
    reported: Option<(u32, u32)>,
    expected: u64,
    lost: u64,
    sent: u64,
    nacked: std::collections::HashSet<u16>,
}

/// Fewer packets than this in a second tell nothing about loss.
const LOSS_SAMPLE: u64 = 20;

impl Loss {
    fn report(&mut self, lost: u32, highest: u32) {
        if let Some((before_lost, before_highest)) = self.reported {
            let expected = highest.wrapping_sub(before_highest);
            // Out of order, or the server started counting again.
            if expected < 1 << 20 {
                self.expected += u64::from(expected);
                self.lost += u64::from(lost.saturating_sub(before_lost).min(expected));
            }
        }
        self.reported = Some((lost, highest));
    }

    /// The fraction of packets lost since the last call, the larger of the
    /// two measures, and starts counting again.
    #[cfg(test)]
    fn take(&mut self) -> Option<f64> {
        self.take_both().0
    }

    /// As [`Self::take`], with each measure: the server's reports of what
    /// reached it from here, and viewers' NACKs, which also count what was
    /// lost on the way to them.
    fn take_both(&mut self) -> (Option<f64>, Option<f64>, Option<f64>) {
        let reported = (self.expected >= LOSS_SAMPLE).then(|| self.lost as f64 / self.expected as f64);
        let nacked = (self.sent >= LOSS_SAMPLE)
            .then(|| (self.nacked.len() as f64 / self.sent as f64).min(1.0));
        self.expected = 0;
        self.lost = 0;
        self.sent = 0;
        self.nacked.clear();
        let larger = match (reported, nacked) {
            (Some(reported), Some(nacked)) => Some(reported.max(nacked)),
            (reported, nacked) => reported.or(nacked),
        };
        (larger, reported, nacked)
    }
}

/// The most bit/s a stream of `height` lines may take, as Discord's client
/// decides it at 60 fps: 3.5 Mbit/s up to 720p, 9 Mbit/s to 1080p, and
/// 20 Mbit/s above, so 1440p keeps its detail. Op 12 announces it.
pub fn max_bitrate(height: u32, fps: u32) -> u32 {
    let base: u64 = match height {
        ..=720 => 3_500_000,
        ..=1080 => 9_000_000,
        _ => 20_000_000,
    };
    // Past 60 fps the cap grows with the rate, to twice at 240 fps and
    // above, so each frame keeps enough bits for small text; at most
    // 40 Mbit/s. Growing to four times sent 1080p144 at 21.6 Mbit/s, past
    // what viewers' connections took: they asked again for a third of the
    // packets while the server got them all. The rate control lowers it.
    (base * u64::from(180 + fps.clamp(60, 240) - 60) / 180).min(40_000_000) as u32
}

/// Op 5's speaking flags for a stream's sound: shared sound, not a
/// microphone, as Discord's client marks it.
const SOUNDSHARE: u8 = 2;

/// The codec every Discord Obsidian sender can stream in, as every other Go Live
/// sender known to work does. A GPU that encodes AV1 or H265 offers those
/// first.
pub const SEND_CODEC: Codec = Codec::H264;

/// One encoded frame to send, in `codec`, with its time in the 90 kHz clock.
pub struct Frame {
    pub data: Vec<u8>,
    pub codec: Codec,
    pub timestamp: u32,
    pub keyframe: bool,
}

/// One Opus frame of the stream's sound: 20 ms of stereo at 48 kHz, with
/// `timestamp` in samples.
pub struct Sound {
    pub opus: Vec<u8>,
    pub timestamp: u32,
}

/// Queues the stream's sound, from a thread of the sender's own.
#[derive(Clone)]
pub struct SoundSender(mpsc::Sender<Sound>);

impl SoundSender {
    /// Queues a frame of sound, dropping it if the network can't keep up.
    pub fn send(&self, sound: Sound) {
        let _ = self.0.try_send(sound);
    }
}

/// One whole frame received, decrypted and ready to decode.
pub struct ReceivedFrame {
    pub codec: Codec,
    pub data: Vec<u8>,
    pub keyframe: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    Ended(String),
}

struct Shared {
    link: Mutex<Link>,
    /// Sending: the next frame should be a keyframe. Receiving: ask the
    /// streamer for one.
    keyframe: AtomicBool,
    size: Mutex<(u32, u32)>,
    /// Sending: the frame rate op 12 announces.
    fps: AtomicU32,
    /// Sending: the bit/s [`RateControl`] aims for, given the loss.
    target: AtomicU64,
    /// Sending: the codecs offered, most wanted first, and whether that
    /// changed since Discord was told. Receiving: the codecs decoded.
    offers: Mutex<Vec<Codec>>,
    offers_changed: AtomicBool,
    /// Sending: the codec Discord chose, in op 4 and then op 14, once it
    /// said.
    chosen: Mutex<Option<Codec>>,
    /// Sending: the video's bytes sent so far, resent ones too, as UDP
    /// payloads; the keyframes viewers asked for; and the packets resent.
    video_bytes: AtomicU64,
    keyframe_requests: AtomicU64,
    resent: AtomicU64,
    /// Sending: frames the encoder skipped while the network was behind,
    /// and encoded frames dropped on a full queue, since last logged.
    skipped: AtomicU32,
    dropped: AtomicU32,
}

impl Shared {
    fn new(size: (u32, u32)) -> Arc<Self> {
        Arc::new(Self {
            link: Mutex::new(Link::Connecting),
            keyframe: AtomicBool::new(true),
            size: Mutex::new(size),
            fps: AtomicU32::new(0),
            target: AtomicU64::new(0),
            offers: Mutex::new(vec![SEND_CODEC]),
            offers_changed: AtomicBool::new(false),
            chosen: Mutex::new(None),
            video_bytes: AtomicU64::new(0),
            keyframe_requests: AtomicU64::new(0),
            resent: AtomicU64::new(0),
            skipped: AtomicU32::new(0),
            dropped: AtomicU32::new(0),
        })
    }
}

/// Runs a connection on a thread of its own until it ends, recording why.
fn spawn(
    name: &str,
    shared: Arc<Shared>,
    run: impl FnOnce(Arc<Shared>) -> std::pin::Pin<Box<dyn Future<Output = Result<()>>>>
    + Send
    + 'static,
) -> Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| runtime.block_on(run(shared.clone())));
            let reason = match result {
                Ok(()) => "stopped".to_owned(),
                Err(error) => {
                    tracing::warn!(error = format!("{error:#}"), "the stream connection ended");
                    format!("{error:#}")
                }
            };
            *lock(&shared.link) = Link::Ended(reason);
        })
        .context("could not start the stream connection")?;
    Ok(())
}

/// The user and channel ids a stream key names, as numbers.
fn ids(server: &StreamServer) -> Result<(u64, u64, u64)> {
    let (_, channel, streamer) = stream::parse(&server.stream_key).context("not a stream key")?;
    Ok((
        server.user_id.parse().context("not a user id")?,
        channel.parse().context("not a channel id")?,
        streamer.parse().context("not a user id")?,
    ))
}

/// Sends frames on a thread of its own; dropping it ends the connection.
pub struct StreamSender {
    frames: mpsc::Sender<Frame>,
    sounds: mpsc::Sender<Sound>,
    shared: Arc<Shared>,
}

impl StreamSender {
    /// Connects to `server` and announces `width` by `height` video,
    /// offering to send in `offers`, most wanted first. Discord picks the
    /// first every viewer can decode; see [`Self::codec`].
    pub fn start(
        server: StreamServer,
        width: u32,
        height: u32,
        fps: u32,
        offers: Vec<Codec>,
    ) -> Result<Self> {
        let (user, channel, _) = ids(&server)?;
        let shared = Shared::new((width, height));
        *lock(&shared.offers) = offers;
        shared.fps.store(fps, Ordering::Relaxed);
        shared.target.store(
            RateControl::new(u64::from(max_bitrate(height, fps))).bitrate(),
            Ordering::Relaxed,
        );
        // About a tenth of a second of frames, and at least four: a keyframe
        // takes a few frames' time to send. Beyond that the network can't
        // keep up.
        let (frames, receiver) = mpsc::channel(frame_queue(fps));
        // 160 ms of sound.
        let (sounds, heard) = mpsc::channel(8);
        spawn("obsidian-stream-send", shared.clone(), move |shared| {
            Box::pin(
                async move { send(&server, user, channel, &shared, receiver, heard).await },
            )
        })?;
        Ok(Self {
            frames,
            sounds,
            shared,
        })
    }

    /// Where the stream's sound goes.
    pub fn sound(&self) -> SoundSender {
        SoundSender(self.sounds.clone())
    }

    /// Queues a frame. A full queue drops it and asks for a keyframe, so
    /// viewers recover once the network catches up.
    pub fn send(&self, frame: Frame) {
        if self.frames.try_send(frame).is_err() {
            self.shared.keyframe.store(true, Ordering::Relaxed);
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Announces a new frame size with the next frame.
    pub fn resize(&self, width: u32, height: u32) {
        *lock(&self.shared.size) = (width, height);
    }

    /// Announces a new frame rate with the next frame, which also changes
    /// the stream's cap.
    pub fn set_fps(&self, fps: u32) {
        self.shared.fps.store(fps, Ordering::Relaxed);
    }

    /// Whether the next frame should be a keyframe: at the start, after
    /// dropped frames, or when a viewer asked for one.
    pub fn take_keyframe_request(&self) -> bool {
        self.shared.keyframe.swap(false, Ordering::Relaxed)
    }

    /// Whether frames wait to be sent: more than half the queue. The
    /// encoder skips a frame then, which, unlike a frame dropped after
    /// encoding, needs no keyframe for viewers to go on; it's counted as
    /// skipped for the log.
    pub fn backlogged(&self) -> bool {
        let queued = self.frames.max_capacity() - self.frames.capacity();
        let behind = queued * 2 > self.frames.max_capacity();
        if behind {
            self.shared.skipped.fetch_add(1, Ordering::Relaxed);
        }
        behind
    }

    /// The bit/s to encode at: the cap for the stream's size and rate,
    /// lowered while packets are lost on the way ([`RateControl`]).
    pub fn target_bitrate(&self) -> u64 {
        self.shared.target.load(Ordering::Relaxed)
    }

    pub fn link(&self) -> Link {
        lock(&self.shared.link).clone()
    }

    /// The codec Discord wants the frames in, once it said. It changes when
    /// a viewer joins who can't decode it, or leaves; frames in another
    /// codec are dropped.
    pub fn codec(&self) -> Option<Codec> {
        let chosen = (*lock(&self.shared.chosen))?;
        let offers = lock(&self.shared.offers);
        // Withdrawn, while Discord hasn't chosen again yet.
        Some(chosen)
            .filter(|chosen| offers.contains(chosen))
            .or(offers.first().copied())
    }

    /// What the stream sent so far, and what viewers asked of it.
    pub fn sent(&self) -> SentSoFar {
        let load = |count: &AtomicU64| count.load(Ordering::Relaxed);
        SentSoFar {
            video_bytes: load(&self.shared.video_bytes),
            keyframe_requests: load(&self.shared.keyframe_requests),
            resent: load(&self.shared.resent),
        }
    }

    /// Stops offering `codec`, which the encoder can't produce any more:
    /// Discord then picks another, which [`Self::codec`] gives.
    pub fn withdraw(&self, codec: Codec) {
        let mut offers = lock(&self.shared.offers);
        if offers.len() > 1 && offers.contains(&codec) {
            offers.retain(|&offered| offered != codec);
            self.shared.offers_changed.store(true, Ordering::Relaxed);
        }
    }
}

/// A stream's video sent since it started, as [`StreamSender::sent`] says:
/// bytes as UDP payloads, resent ones too; keyframes viewers asked for; and
/// packets resent when a viewer reported them lost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SentSoFar {
    pub video_bytes: u64,
    pub keyframe_requests: u64,
    pub resent: u64,
}

/// Receives someone's stream on a thread of its own; dropping it ends the
/// connection.
pub struct StreamReceiver {
    frames: channel::Receiver<ReceivedFrame>,
    sounds: Option<channel::Receiver<Vec<u8>>>,
    shared: Arc<Shared>,
    _stop: oneshot::Sender<()>,
}

impl StreamReceiver {
    /// Watches the stream, telling Discord the viewer decodes `decodes`,
    /// most wanted first.
    pub fn start(server: StreamServer, decodes: Vec<Codec>) -> Result<Self> {
        let (user, channel, streamer) = ids(&server)?;
        let shared = Shared::new((0, 0));
        *lock(&shared.offers) = decodes;
        // Frames waiting to be decoded; more means decoding can't keep up.
        let (frames, receiver) = channel::sync_channel(4);
        // Opus frames waiting to be played: 320 ms.
        let (sounds, heard) = channel::sync_channel(16);
        let (stop, stopped) = oneshot::channel();
        spawn("obsidian-stream-receive", shared.clone(), move |shared| {
            Box::pin(async move {
                receive_stream(
                    &server,
                    user,
                    channel,
                    streamer,
                    &shared,
                    (frames, sounds),
                    stopped,
                )
                .await
            })
        })?;
        Ok(Self {
            frames: receiver,
            sounds: Some(heard),
            shared,
            _stop: stop,
        })
    }

    /// The next whole frame, waiting at most `wait`.
    pub fn next(&self, wait: Duration) -> Option<ReceivedFrame> {
        self.frames.recv_timeout(wait).ok()
    }

    /// The stream's sound, decrypted Opus frames, for a player of its own.
    /// It ends with the connection.
    pub fn take_sounds(&mut self) -> Option<channel::Receiver<Vec<u8>>> {
        self.sounds.take()
    }

    /// Asks the streamer for a keyframe, as after a frame that wouldn't decode.
    pub fn request_keyframe(&self) {
        self.shared.keyframe.store(true, Ordering::Relaxed);
    }

    pub fn link(&self) -> Link {
        lock(&self.shared.link).clone()
    }
}

pub(crate) fn video_attributes(
    ssrc: u32,
    video: u32,
    rtx: u32,
    (width, height): (u32, u32),
    fps: u32,
) -> Value {
    json!({
        "audio_ssrc": ssrc,
        "video_ssrc": video,
        "rtx_ssrc": rtx,
        "streams": [{
            "type": "video",
            "rid": rtp::STREAM_RID,
            "ssrc": video,
            "active": true,
            "quality": 100,
            "rtx_ssrc": rtx,
            "max_bitrate": max_bitrate(height, fps),
            "max_framerate": fps,
            "max_resolution": {"type": "fixed", "width": width, "height": height},
        }],
    })
}

/// Which way the video goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Send,
    Receive,
}

/// Connects to a stream's server: identifying with its RTC server and a
/// screen, and offering the codecs `role` needs.
/// Select protocol's codecs for a sender: Opus, then each of `offers` by
/// priority, and `withdrawn` ones it can't encode any more. A sender
/// decodes nothing of its own stream.
fn send_codecs(offers: &[Codec], withdrawn: &[Codec]) -> Value {
    let video = offers
        .iter()
        .map(|&codec| (codec, true))
        .chain(withdrawn.iter().map(|&codec| (codec, false)));
    codecs(video.map(|(codec, encode)| (codec, encode, matches!(codec, Codec::Vp8 | Codec::H264))))
}

/// Select protocol's codecs for a viewer: Opus, then each of `decodes` by
/// priority, to decode only.
fn receive_codecs(decodes: &[Codec]) -> Value {
    codecs(decodes.iter().map(|&codec| (codec, false, true)))
}

/// Opus, then each video codec with whether it's encoded and decoded, most
/// wanted first.
fn codecs(video: impl Iterator<Item = (Codec, bool, bool)>) -> Value {
    let mut codecs =
        vec![json!({"name": "opus", "type": "audio", "priority": 1000, "payload_type": rtp::OPUS})];
    for (index, (codec, encode, decode)) in video.enumerate() {
        let (payload_type, rtx_payload_type) = codec.payload_types();
        codecs.push(json!({
            "name": codec.name(), "type": "video", "priority": 1000 * (index + 1),
            "payload_type": payload_type, "rtx_payload_type": rtx_payload_type,
            "encode": encode, "decode": decode,
        }));
    }
    Value::Array(codecs)
}

async fn open(
    server: &StreamServer,
    user: u64,
    channel: u64,
    role: Role,
    shared: &Shared,
) -> Result<Connection> {
    // Discord made a stream's DAVE group id just before its RTC server id,
    // in the one stream seen so far; Dave moves to the id the voice server
    // uses if this guess is wrong.
    let group = server
        .rtc_server_id
        .parse::<u64>()
        .map_or(channel, |id| id.saturating_sub(1));
    let identity = Identity {
        endpoint: server.endpoint.clone(),
        server_id: server.rtc_server_id.clone(),
        user_id: server.user_id.clone(),
        session_id: server.session_id.clone(),
        token: server.token.clone(),
        streams: json!([{"type": "screen", "rid": rtp::STREAM_RID, "quality": 100}]),
        user,
        group,
        ping: Default::default(),
    };
    // Discord Obsidian encodes in what it offers, and decodes what it was given.
    let codecs = match role {
        Role::Send => send_codecs(&lock(&shared.offers), &[]),
        Role::Receive => receive_codecs(&lock(&shared.offers)),
    };
    let streamer: Vec<u64> = stream::parse(&server.stream_key)
        .and_then(|(_, _, streamer)| streamer.parse().ok())
        .into_iter()
        .collect();
    let connection = Connection::open(&identity, codecs, &streamer).await?;
    tracing::info!(rtc_server = %server.rtc_server_id, channel, "connected to a Discord stream");
    Ok(connection)
}

async fn send(
    server: &StreamServer,
    user: u64,
    channel: u64,
    shared: &Shared,
    mut frames: mpsc::Receiver<Frame>,
    mut sounds: mpsc::Receiver<Sound>,
) -> Result<()> {
    let mut connection = open(server, user, channel, Role::Send, shared).await?;
    let (ssrc, video, rtx) = (connection.ssrc, connection.video, connection.rtx);
    let mut size = *lock(&shared.size);
    let mut fps = shared.fps.load(Ordering::Relaxed);
    send_json(
        &mut connection.ws,
        12,
        video_attributes(ssrc, video, rtx, size, fps),
    )
    .await?;
    let mut rate = RateControl::new(u64::from(max_bitrate(size.1, fps)));
    // The delay-based estimate from the server's transport-wide feedback,
    // and when each packet went, to match it to. The stream aims for the
    // lower of it and the loss-based one, as libwebrtc does.
    let mut delay = DelayBased::new(rate.bitrate());
    // Whether the server sends that feedback: until it does, the loss-based
    // estimate alone decides, so a server that never does can't hold the
    // stream at where the delay-based one starts.
    let mut delay_heard = false;
    let mut sent_at = SendHistory::default();
    let clock = std::time::Instant::now();
    let micros = move || clock.elapsed().as_micros() as i64;
    // The target last logged.
    let mut logged = rate.bitrate();
    // When the pacer may send the next packet: kept across frames, so a
    // frame queued behind a keyframe waits its turn.
    let mut due = Instant::now();
    let mut loss = Loss::default();
    // The bytes resent since the rate control last looked.
    let mut resent_bytes: u64 = 0;
    let mut rated = tokio::time::interval_at(Instant::now() + RATE_EVERY, RATE_EVERY);
    // Frames skipped or dropped are logged at most this often, summed.
    let mut held_back = tokio::time::interval_at(Instant::now() + HELD_BACK_EVERY, HELD_BACK_EVERY);
    send_json(
        &mut connection.ws,
        5,
        json!({"speaking": SOUNDSHARE, "delay": 0, "ssrc": ssrc}),
    )
    .await?;
    *lock(&shared.link) = Link::Connected;

    // Discord's choice, or the first offered while it hasn't said.
    let offered = lock(&shared.offers).clone();
    let chosen = connection.codec.unwrap_or(offered[0]);
    if !offered.contains(&chosen) {
        bail!("Discord asked for {chosen:?} video, and Discord Obsidian offered {offered:?}");
    }
    *lock(&shared.chosen) = Some(chosen);
    tracing::info!(codec = ?chosen, ?offered, "Discord chose the stream's codec");
    let mut withdrawn: Vec<Codec> = vec![];
    let mut packetizer = Packetizer::new(video, chosen);
    let mut history = rtp::History::default();
    let interval = connection.interval;
    let mut heartbeat = tokio::time::interval_at(Instant::now() + interval, interval);
    let mut datagram = vec![0u8; 1500];
    // For RTCP sender reports: packets and payload bytes sent so far.
    let (mut packets, mut octets) = (0u32, 0u32);
    let mut reported = Instant::now() - SENDER_REPORTS;
    // For the log: whether frames went out encrypted, and what came back.
    let mut encrypted = None;
    let mut sent = Sent::default();
    let mut log = tokio::time::interval_at(Instant::now() + LOG_EVERY, LOG_EVERY);
    // The sound's RTP numbering and sender report counts, on the audio SSRC.
    let mut sound_sequence: u16 = rand::random();
    let (mut sound_packets, mut sound_octets) = (0u32, 0u32);
    let mut sound_reported = Instant::now() - SENDER_REPORTS;
    loop {
        tokio::select! {
            frame = frames.recv() => {
                let Some(frame) = frame else {
                    let _ = connection.ws.close(None).await;
                    return Ok(());
                };
                let wanted = (*lock(&shared.size), shared.fps.load(Ordering::Relaxed));
                if wanted != (size, fps) {
                    (size, fps) = wanted;
                    send_json(&mut connection.ws, 12, video_attributes(ssrc, video, rtx, size, fps)).await?;
                    let cap = u64::from(max_bitrate(size.1, fps));
                    rate.cap(cap);
                    let delayed = if delay_heard { delay.bitrate() } else { cap };
                    shared.target.store(rate.bitrate().min(delayed).min(cap), Ordering::Relaxed);
                }
                let encrypting = connection.dave.encrypting();
                if encrypted != Some(encrypting) {
                    encrypted = Some(encrypting);
                    tracing::info!(
                        end_to_end = encrypting,
                        dave = connection.dave.status(),
                        "sending stream frames"
                    );
                }
                if shared.offers_changed.swap(false, Ordering::Relaxed) {
                    let offers = lock(&shared.offers).clone();
                    withdrawn = offered.iter().copied().filter(|codec| !offers.contains(codec)).collect();
                    tracing::info!(?offers, ?withdrawn, "offering fewer codecs");
                    send_json(&mut connection.ws, 14, json!({"codecs": send_codecs(&offers, &withdrawn)})).await?;
                }
                // Frames encoded before the encoder followed Discord's choice.
                if Some(frame.codec) != *lock(&shared.chosen) {
                    continue;
                }
                packetizer.set_codec(frame.codec);
                let Some(payload) = connection.dave.encrypt(frame.codec, &frame.data) else {
                    continue;
                };
                sent.frames += 1;
                sent.keyframes += u32::from(frame.keyframe);
                // Paced on a schedule in absolute time: a timer that wakes
                // late, as one does at Windows' default 15.6 ms tick, sends
                // the packets due meanwhile at once instead of holding the
                // stream to a few frames a second.
                let paced = pace(shared.target.load(Ordering::Relaxed));
                let now = Instant::now();
                let idle = due <= now;
                if idle {
                    due = now;
                }
                for (index, mut packet) in packetizer.packets(&payload, frame.timestamp, frame.keyframe).into_iter().enumerate() {
                    if !idle || index >= BURST {
                        due += airtime(packet.len(), paced);
                        if due > Instant::now() + Duration::from_millis(1) {
                            tokio::time::sleep_until(due).await;
                        }
                    }
                    loss.sent += 1;
                    history.remember(&packet);
                    let transport_sequence = rtp::transport_sequence(&packet);
                    let clear = rtp::rtp_clear_len(&packet).unwrap_or(packet.len());
                    packets = packets.wrapping_add(1);
                    octets = octets.wrapping_add((packet.len() - clear) as u32);
                    connection.transport.seal(&mut packet, clear)?;
                    crate::voice_connection::send_media(&connection.udp, &packet).await?;
                    if let Some(sequence) = transport_sequence {
                        sent_at.record(sequence, micros(), packet.len() as u32);
                    }
                    shared.video_bytes.fetch_add(packet.len() as u64, Ordering::Relaxed);
                }
                if reported.elapsed() >= SENDER_REPORTS {
                    reported = Instant::now();
                    let mut report = rtp::sender_report(
                        video,
                        std::time::SystemTime::now(),
                        frame.timestamp,
                        packets,
                        octets,
                    );
                    connection.transport.seal(&mut report, rtp::RTCP_CLEAR)?;
                    crate::voice_connection::send_media(&connection.udp, &report).await?;
                }
            }
            _ = heartbeat.tick() => connection.heartbeat().await?,
            message = receive(&mut connection.ws, &mut connection.seq) => {
                // Not heartbeat acks, nor the keys in the ready message and
                // session description.
                if let Some((op, data)) = connection.handle(message?).await?
                    && !matches!(op, 2 | 4 | 6)
                {
                    let mut text = data.to_string();
                    text.truncate(400);
                    tracing::debug!(op, data = %text, "the stream server said");
                    // Session update: a viewer can't decode the codec, or the
                    // one who couldn't left.
                    if op == 14
                        && let Some(codec) = data["video_codec"].as_str().and_then(Codec::from_name)
                    {
                        if offered.contains(&codec) && !withdrawn.contains(&codec) {
                            tracing::info!(?codec, "Discord switched the stream's codec");
                            *lock(&shared.chosen) = Some(codec);
                            shared.keyframe.store(true, Ordering::Relaxed);
                        } else {
                            tracing::warn!(?codec, "Discord asked for a codec Discord Obsidian didn't offer");
                        }
                    }
                    // Tell whoever connected about the video again, in case
                    // the server doesn't pass it on to late viewers.
                    if op == 11 {
                        send_json(&mut connection.ws, 12, video_attributes(ssrc, video, rtx, size, fps)).await?;
                        send_json(&mut connection.ws, 5, json!({"speaking": SOUNDSHARE, "delay": 0, "ssrc": ssrc})).await?;
                    }
                }
            }
            Some(sound) = sounds.recv() => {
                let Some(payload) = connection.dave.encrypt_opus(&sound.opus) else {
                    continue;
                };
                let mut packet =
                    rtp::opus_packet(ssrc, sound_sequence, sound.timestamp, sound_packets == 0, &payload);
                sound_sequence = sound_sequence.wrapping_add(1);
                sound_packets = sound_packets.wrapping_add(1);
                sound_octets = sound_octets.wrapping_add(payload.len() as u32);
                let clear = rtp::rtp_clear_len(&packet).unwrap_or(packet.len());
                connection.transport.seal(&mut packet, clear)?;
                crate::voice_connection::send_media(&connection.udp, &packet).await?;
                sent.sounds += 1;
                if sound_reported.elapsed() >= SENDER_REPORTS {
                    sound_reported = Instant::now();
                    let mut report = rtp::sender_report(
                        ssrc,
                        std::time::SystemTime::now(),
                        sound.timestamp,
                        sound_packets,
                        sound_octets,
                    );
                    connection.transport.seal(&mut report, rtp::RTCP_CLEAR)?;
                    crate::voice_connection::send_media(&connection.udp, &report).await?;
                }
            }
            received = connection.udp.recv(&mut datagram) => {
                let packet = &datagram[..received?];
                if !rtp::is_rtcp(packet) {
                    sent.rtp_back += 1;
                    continue;
                }
                let Some(rtcp) = connection.transport.open(packet, rtp::RTCP_CLEAR) else {
                    sent.unopened += 1;
                    continue;
                };
                for (kind, said) in rtp::rtcp_kinds(&rtcp) {
                    *sent.rtcp.entry(kind.clone()).or_default() += 1;
                    if !said.is_empty() {
                        sent.said.insert(kind, said);
                    }
                }
                if rtp::wants_keyframe(&rtcp) {
                    sent.requests += 1;
                    shared.keyframe_requests.fetch_add(1, Ordering::Relaxed);
                    shared.keyframe.store(true, Ordering::Relaxed);
                }
                if let Some(estimate) = rtp::bandwidth_estimate(&rtcp, video) {
                    sent.estimate = estimate;
                }
                let feedback = congestion::transport_feedback(&rtcp);
                if !feedback.is_empty() {
                    let cap = u64::from(max_bitrate(size.1, fps));
                    for report in &feedback {
                        delay.update(&sent_at.arrivals(report), micros(), MIN_BITRATE, cap);
                    }
                    sent.feedback += feedback.len() as u32;
                    if !delay_heard {
                        delay_heard = true;
                        tracing::info!("following the server's transport-wide feedback");
                    }
                    shared.target.store(rate.bitrate().min(delay.bitrate()), Ordering::Relaxed);
                }
                for (reported, lost, highest) in rtp::report_blocks(&rtcp) {
                    if reported == video {
                        loss.report(lost, highest);
                    }
                }
                let nacked = rtp::nacked(&rtcp, video);
                loss.nacked.extend(nacked.iter().copied());
                for lost in nacked.into_iter().take(RESENDS) {
                    if resent_bytes * 8 > shared.target.load(Ordering::Relaxed) * RATE_EVERY.as_secs() / RESEND_SHARE {
                        sent.over_budget += 1;
                        continue;
                    }
                    let Some(mut again) = history.get(lost).and_then(|original| packetizer.retransmit(original, rtx)) else {
                        sent.too_late += 1;
                        continue;
                    };
                    resent_bytes += again.len() as u64;
                    let transport_sequence = rtp::transport_sequence(&again);
                    let clear = rtp::rtp_clear_len(&again).unwrap_or(again.len());
                    connection.transport.seal(&mut again, clear)?;
                    crate::voice_connection::send_media(&connection.udp, &again).await?;
                    if let Some(sequence) = transport_sequence {
                        sent_at.record(sequence, micros(), again.len() as u32);
                    }
                    shared.video_bytes.fetch_add(again.len() as u64, Ordering::Relaxed);
                    shared.resent.fetch_add(1, Ordering::Relaxed);
                    sent.resent += 1;
                }
            }
            _ = rated.tick() => {
                resent_bytes = 0;
                let (lost, upload, viewers) = loss.take_both();
                rate.update(lost, u64::from(max_bitrate(size.1, fps)), delay.acked());
                let after = if delay_heard { rate.bitrate().min(delay.bitrate()) } else { rate.bitrate() };
                shared.target.store(after, Ordering::Relaxed);
                // A change of a tenth or more, not every step of 8%.
                if logged.abs_diff(after) > logged / 10 {
                    logged = after;
                    let percent = |loss: Option<f64>| loss.map(|loss| (loss * 1000.0).round() / 10.0);
                    tracing::info!(
                        kbps = after / 1000,
                        delay_kbps = delay.bitrate() / 1000,
                        loss_kbps = rate.bitrate() / 1000,
                        delay = ?delay.signal(),
                        acked_kbps = delay.acked().map(|acked| (acked / 1000.0) as u64),
                        loss_percent = percent(lost),
                        upload_loss_percent = percent(upload),
                        viewer_nack_percent = percent(viewers),
                        "stream bitrate"
                    );
                }
            }
            _ = held_back.tick() => {
                let skipped = shared.skipped.swap(0, Ordering::Relaxed);
                let dropped = shared.dropped.swap(0, Ordering::Relaxed);
                if skipped > 0 || dropped > 0 {
                    tracing::info!(
                        skipped,
                        dropped,
                        seconds = HELD_BACK_EVERY.as_secs(),
                        kbps = shared.target.load(Ordering::Relaxed) / 1000,
                        "stream frames held back while the network was behind"
                    );
                }
            }
            _ = log.tick() => {
                tracing::debug!(
                    ?sent,
                    packets,
                    octets,
                    target = shared.target.load(Ordering::Relaxed),
                    delay_based = delay.bitrate(),
                    loss_based = rate.bitrate(),
                    "stream sent"
                );
                sent = Sent::default();
            }
        }
    }
}

/// How often the rate control looks at the loss, about as often as the
/// server's receiver reports come.
const RATE_EVERY: Duration = Duration::from_secs(1);
/// How often frames skipped or dropped are logged, when there were any.
const HELD_BACK_EVERY: Duration = Duration::from_secs(10);

/// How many encoded frames may wait to be sent at `fps`: about a tenth of a
/// second's, and at least four.
fn frame_queue(fps: u32) -> usize {
    (fps as usize / 10).clamp(4, 36)
}

/// What a streamer sent and heard back since it last logged.
#[derive(Debug, Default)]
struct Sent {
    frames: u32,
    keyframes: u32,
    /// Opus frames of the stream's sound.
    sounds: u32,
    /// RTCP by packet type and feedback format, as `type/format`.
    rtcp: std::collections::BTreeMap<String, u32>,
    /// What the last of each kind said.
    said: std::collections::BTreeMap<String, String>,
    /// Keyframe requests among them.
    requests: u32,
    /// Packets resent when a viewer reported them lost, and those already
    /// gone from the history.
    resent: u32,
    too_late: u32,
    /// Packets asked for again past the share resends may take.
    over_budget: u32,
    /// Transport-wide feedback reports the server sent.
    feedback: u32,
    /// RTCP that wouldn't decrypt.
    unopened: u32,
    /// RTP, which a streamer isn't sent.
    rtp_back: u32,
    /// Discord's server's last estimate of the path (REMB) in bit/s, logged
    /// only: see [`RateControl`].
    estimate: u64,
}

/// What the server said about the streamer's video.
#[derive(Default)]
struct Streamer {
    video: Option<u32>,
    rtx: Option<u32>,
    codec: Option<Codec>,
}

impl Streamer {
    fn learn(&mut self, op: u64, data: &Value) {
        match op {
            // Video: the streamer's SSRCs.
            12 => {
                let ssrc = |value: &Value| {
                    value
                        .as_u64()
                        .filter(|&ssrc| ssrc != 0)
                        .map(|ssrc| ssrc as u32)
                };
                self.video = ssrc(&data["video_ssrc"]).or(self.video);
                self.rtx = ssrc(&data["rtx_ssrc"]).or(self.rtx);
            }
            // Session update: the codec may change.
            14 => {
                if let Some(codec) = data["video_codec"].as_str().and_then(Codec::from_name) {
                    self.codec = Some(codec);
                }
            }
            _ => {}
        }
    }

    /// The codec of a packet, by its payload type or, from the streamer's
    /// video SSRC, by what Discord chose.
    fn codec(&self, packet: &rtp::Packet) -> Option<Codec> {
        if Some(packet.ssrc) == self.rtx {
            return None;
        }
        Codec::from_payload_type(packet.payload_type).or_else(|| {
            (Some(packet.ssrc) == self.video)
                .then_some(self.codec)
                .flatten()
        })
    }
}

/// What a viewer's connection has received, logged while no picture
/// reaches the decoder so a stuck stream shows where it stops.
#[derive(Debug, Default)]
struct Received {
    packets: u64,
    rtcp: u64,
    unopened: u64,
    unparsed: u64,
    /// Packets of a payload type that isn't the streamer's video.
    other: u64,
    video: u64,
    frames: u64,
    unreadable: u64,
    decoded: u64,
    /// Opus frames of the stream's sound, and those that wouldn't decrypt.
    sounds: u64,
    unheard: u64,
    /// Packets on the RTX SSRC: resent video, and padding.
    resent: u64,
    padding: u64,
    /// Video packets that came out of order, those that came after they
    /// were asked for again, those that came too late or twice, and those
    /// never seen.
    reordered: u64,
    recovered: u64,
    late: u64,
    given_up: u64,
    /// Packets asked for again (NACK), and keyframes asked for (PLI).
    asked: u64,
    keyframe_requests: u64,
    /// Frames dropped because the decoder was behind.
    behind: u64,
    payload_types: std::collections::BTreeSet<u8>,
    ssrcs: std::collections::BTreeSet<u32>,
}

/// One SSRC's video on its way to frames.
struct Video {
    reorder: rtp::Reorder,
    assembler: Assembler,
    /// For receiver reports: the counts at the last one.
    reported: (u32, u32),
}

async fn receive_stream(
    server: &StreamServer,
    user: u64,
    channel: u64,
    streamer_id: u64,
    shared: &Shared,
    // Where whole video frames and the sound's Opus frames go.
    (frames, sounds): (
        channel::SyncSender<ReceivedFrame>,
        channel::SyncSender<Vec<u8>>,
    ),
    mut stopped: oneshot::Receiver<()>,
) -> Result<()> {
    let mut connection = open(server, user, channel, Role::Receive, shared).await?;
    let mut streamer = Streamer {
        codec: connection.codec,
        ..Streamer::default()
    };
    for (op, data) in std::mem::take(&mut connection.early) {
        streamer.learn(op, &data);
    }
    // Media sink wants: the server forwards video at the quality a client
    // asks for, here everything at full quality.
    send_json(&mut connection.ws, 15, json!({"any": 100})).await?;
    *lock(&shared.link) = Link::Connected;

    let mut incoming: HashMap<u32, Video> = HashMap::new();
    let mut asked = Instant::now() - KEYFRAME_REQUESTS;
    let interval = connection.interval;
    let mut heartbeat = tokio::time::interval_at(Instant::now() + interval, interval);
    let mut keyframes = tokio::time::interval(KEYFRAME_REQUESTS);
    // Gives up on missing packets and asks for them again even while
    // nothing arrives.
    let mut repairs = tokio::time::interval(Duration::from_millis(20));
    let mut reports = tokio::time::interval(Duration::from_secs(1));
    // The streamer's last sender report, as its NTP time's middle and when
    // it came, which receiver reports echo for the round trip.
    let mut last_report: Option<(u32, std::time::Instant)> = None;
    let mut datagram = vec![0u8; 2048];
    let mut received = Received::default();
    let mut log = tokio::time::interval_at(Instant::now() + LOG_EVERY, LOG_EVERY);
    // Frames handed to the decoder when the log last looked.
    let mut flowing = 0;
    loop {
        let mut lost = false;
        tokio::select! {
            _ = &mut stopped => {
                let _ = connection.ws.close(None).await;
                return Ok(());
            }
            _ = heartbeat.tick() => connection.heartbeat().await?,
            message = receive(&mut connection.ws, &mut connection.seq) => {
                if let Some((op, data)) = connection.handle(message?).await? {
                    if !matches!(op, 2 | 4 | 6) {
                        let mut text = data.to_string();
                        text.truncate(400);
                        tracing::debug!(op, data = %text, "the stream server said");
                    }
                    streamer.learn(op, &data);
                }
            }
            _ = keyframes.tick() => {}
            _ = repairs.tick() => {}
            _ = reports.tick() => {
                let Some(media) = streamer.video else { continue };
                let Some(video) = incoming.get_mut(&media) else { continue };
                let Some((highest, lost)) = video.reorder.counts() else { continue };
                let (expected, missed) = (
                    highest.wrapping_sub(video.reported.0),
                    lost.saturating_sub(video.reported.1),
                );
                let fraction = (u64::from(missed) * 256 / u64::from(expected.max(1))).min(255) as u8;
                video.reported = (highest, lost);
                let (middle, delay) = last_report.map_or((0, 0), |(middle, heard)| {
                    (middle, (heard.elapsed().as_secs_f64() * 65536.0) as u32)
                });
                let mut report = rtp::receiver_report(connection.ssrc, media, (highest, lost, fraction), (middle, delay));
                connection.transport.seal(&mut report, rtp::RTCP_CLEAR)?;
                crate::voice_connection::send_media(&connection.udp, &report).await?;
            }
            _ = log.tick() => {
                tracing::debug!(?received, "stream received");
                if received.decoded == flowing {
                    tracing::info!(
                        ?received,
                        dave = connection.dave.status(),
                        unreadable = connection.dave.unreadable.as_deref(),
                        streamer_video = ?streamer.video,
                        streamer_codec = ?streamer.codec,
                        "no stream frames reached the decoder lately"
                    );
                } else if flowing == 0 {
                    tracing::info!(frames = received.decoded, "stream frames are reaching the decoder");
                }
                flowing = received.decoded;
            }
            length = connection.udp.recv(&mut datagram) => {
                let packet = &datagram[..length?];
                received.packets += 1;
                if rtp::is_rtcp(packet) {
                    received.rtcp += 1;
                    let rtcp = connection.transport.open(packet, rtp::RTCP_CLEAR).unwrap_or_default();
                    for (sender, middle) in rtp::sender_reports(&rtcp) {
                        if Some(sender) == streamer.video {
                            last_report = Some((middle, std::time::Instant::now()));
                        }
                    }
                    continue;
                }
                let Some(clear) = rtp::rtp_clear_len(packet) else {
                    received.unparsed += 1;
                    continue;
                };
                let Some(plain) = connection.transport.open(packet, clear) else {
                    received.unopened += 1;
                    continue;
                };
                let Some(packet) = rtp::parse(&plain) else {
                    received.unparsed += 1;
                    continue;
                };
                received.payload_types.insert(packet.payload_type);
                if received.ssrcs.len() < 64 {
                    received.ssrcs.insert(packet.ssrc);
                }
                if packet.payload_type == rtp::OPUS {
                    received.sounds += 1;
                    match connection.dave.decrypt_opus(streamer_id, packet.payload.to_vec()) {
                        // A full queue drops sound the player can't keep up with.
                        Some(sound) => drop(sounds.try_send(sound)),
                        None => received.unheard += 1,
                    }
                    continue;
                }
                let held = if streamer.rtx.is_some() && Some(packet.ssrc) == streamer.rtx {
                    received.resent += 1;
                    match streamer.video.and_then(|video| rtp::Held::unwrap_rtx(&packet, video)) {
                        Some(held) => held,
                        None => {
                            received.padding += 1;
                            continue;
                        }
                    }
                } else {
                    rtp::Held::new(&packet)
                };
                let Some(codec) = streamer.codec(&held.packet()) else {
                    received.other += 1;
                    continue;
                };
                received.video += 1;
                streamer.video.get_or_insert(held.ssrc);
                // Packets can name any SSRC: past a few, a new one isn't
                // given buffers of its own.
                if incoming.len() >= MAX_SSRCS && !incoming.contains_key(&held.ssrc) {
                    received.other += 1;
                    continue;
                }
                let video = incoming.entry(held.ssrc).or_insert_with(|| Video {
                    reorder: rtp::Reorder::default(),
                    assembler: Assembler::new(codec),
                    reported: (0, 0),
                });
                if video.assembler.codec() != codec {
                    video.assembler = Assembler::new(codec);
                }
                let resent = Some(packet.ssrc) == streamer.rtx;
                match video.reorder.insert(held, std::time::Instant::now()) {
                    rtp::Arrival::New if !resent => {}
                    rtp::Arrival::New | rtp::Arrival::Recovered if resent => received.recovered += 1,
                    rtp::Arrival::New | rtp::Arrival::Recovered => received.reordered += 1,
                    rtp::Arrival::Late => received.late += 1,
                }
            }
        }
        let now = std::time::Instant::now();
        for (&ssrc, video) in &mut incoming {
            let codec = video.assembler.codec();
            let (ready, skipped) = video.reorder.ready(now);
            received.given_up += skipped;
            for held in ready {
                let pushed = video.assembler.push(&held.packet());
                lost |= pushed.lost;
                let Some(frame) = pushed.frame else { continue };
                received.frames += 1;
                let Some(data) = connection.dave.decrypt(streamer_id, frame.data) else {
                    received.unreadable += 1;
                    continue;
                };
                received.decoded += 1;
                let frame = ReceivedFrame {
                    codec,
                    data,
                    keyframe: frame.keyframe,
                };
                if let Err(channel::TrySendError::Full(_)) = frames.try_send(frame) {
                    // The decoder is behind; the frames after this one
                    // need it, so wait for a keyframe.
                    received.behind += 1;
                    video.assembler.wait_for_keyframe();
                    lost = true;
                }
            }
            let missing = video.reorder.to_ask(now);
            if !missing.is_empty() {
                received.asked += missing.len() as u64;
                let mut request = rtp::nack(connection.ssrc, ssrc, &missing);
                connection.transport.seal(&mut request, rtp::RTCP_CLEAR)?;
                crate::voice_connection::send_media(&connection.udp, &request).await?;
            }
        }
        // Ask for a keyframe after a loss, or when the decoder asked.
        if (lost || shared.keyframe.load(Ordering::Relaxed))
            && asked.elapsed() >= KEYFRAME_REQUESTS
            && let Some(media) = streamer.video
        {
            shared.keyframe.store(false, Ordering::Relaxed);
            asked = Instant::now();
            received.keyframe_requests += 1;
            let mut request = rtp::picture_loss(connection.ssrc, media);
            connection.transport.seal(&mut request, rtp::RTCP_CLEAR)?;
            crate::voice_connection::send_media(&connection.udp, &request).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bitrate_starts_where_discords_estimate_does_and_follows_the_loss() {
        let cap = u64::from(max_bitrate(1080, 144));
        let mut rate = RateControl::new(cap);
        assert_eq!(rate.bitrate(), START_BITRATE, "starts where Discord's estimate does");
        assert_eq!(RateControl::new(3_500_000).bitrate(), 3_500_000, "or lower, at 720p30's cap");
        assert_eq!(rate.update(None, cap, None), START_BITRATE, "no reports hold it");
        assert_eq!(rate.update(Some(0.0), cap, None), 4_321_000, "a clear path grows it 8%");
        for _ in 0..60 {
            rate.update(Some(0.0), cap, None);
        }
        assert_eq!(rate.bitrate(), cap, "up to the cap, in about 20 s");
        assert_eq!(rate.update(Some(0.05), cap, None), cap, "some loss holds it");
        assert_eq!(rate.update(Some(0.30), cap, None), cap * 85 / 100, "heavy loss lowers it");
        let lowered = rate.bitrate();
        assert_eq!(
            rate.update(Some(0.01), cap, None),
            lowered * 101 / 100 + 1000,
            "and it grows back slowly near where loss came"
        );
        for _ in 0..60 {
            rate.update(Some(0.0), cap, None);
        }
        assert_eq!(rate.bitrate(), (cap as f64 * CEILING_HOLD) as u64, "but stays under it");
        for _ in 0..RAISE_CEILING {
            rate.update(Some(0.0), cap, None);
        }
        assert!(rate.bitrate() > (cap as f64 * CEILING_HOLD) as u64, "until the path held a while");
        rate.update(Some(0.5), cap, None);
        let far = rate.update(Some(0.5), cap, None);
        assert_eq!(rate.update(Some(0.0), cap, None), far * 108 / 100 + 1000, "far below it, at full speed");
        for _ in 0..100 {
            rate.update(Some(1.0), cap, None);
        }
        assert_eq!(rate.bitrate(), MIN_BITRATE, "a screen's floor");
        rate = RateControl::new(cap);
        rate.cap(3_000_000);
        assert_eq!(rate.bitrate(), 3_000_000, "a smaller quality's cap");
    }

    #[test]
    fn the_bitrate_grows_only_while_the_encoder_uses_it() {
        let cap = u64::from(max_bitrate(1080, 144));
        let mut rate = RateControl::new(cap);
        for _ in 0..60 {
            rate.update(Some(0.0), cap, Some(2_000_000.0));
        }
        assert_eq!(rate.bitrate(), START_BITRATE, "a calm screen sending 2 Mbit/s holds it");
        for _ in 0..60 {
            rate.update(Some(0.0), cap, Some(8_000_000.0));
        }
        assert_eq!(rate.bitrate(), 12_010_000, "one and a half times what gets through");
        assert_eq!(max_bitrate(1080, 144), 13_200_000);
        assert_eq!(max_bitrate(1080, 60), 9_000_000);
        assert_eq!(max_bitrate(1080, 240), 18_000_000);
    }

    #[test]
    fn loss_comes_from_reports_and_nacks() {
        let mut loss = Loss::default();
        assert_eq!(loss.take(), None, "nothing to tell");
        loss.report(0, 1000);
        loss.report(10, 1100);
        assert_eq!(loss.take(), Some(0.1), "10 of 100 lost");
        loss.report(10, 1200);
        loss.sent = 100;
        loss.nacked.extend([1, 2, 3, 3, 3]);
        assert_eq!(loss.take(), Some(0.03), "each NACKed packet once");
        loss.report(10, 1205);
        assert_eq!(loss.take(), None, "too few packets");
        loss.report(10, 5);
        loss.report(12, 105);
        assert_eq!(loss.take(), Some(0.02), "a server counting again starts over");
    }

    #[test]
    fn paces_keyframes_without_holding_the_frame_rate() {
        assert_eq!(pace(100_000), PACE_FLOOR);
        assert_eq!(pace(10_000_000), 12_500_000, "a quarter more than the target");
        // The largest frame the encoder's eighth of a second allows leaves in
        // 100 ms, whatever the bitrate; Discord's 20 KB at 1.5 Mbit/s took 86.
        for target in [1_500_000u64, 4_000_000, 10_000_000, 21_600_000] {
            let largest = (target / 8 / 8) as usize;
            let frame: Duration = (0..largest / 1200).map(|_| airtime(1200, pace(target))).sum();
            assert!(frame <= Duration::from_millis(101), "{target}: {frame:?}");
        }
        assert_eq!(frame_queue(30), 4);
        assert_eq!(frame_queue(144), 14);
        assert_eq!(frame_queue(360), 36);
    }

    #[test]
    fn learns_the_streamers_video() {
        let mut streamer = Streamer::default();
        streamer.learn(
            12,
            &json!({"user_id": "2", "audio_ssrc": 10, "video_ssrc": 11, "rtx_ssrc": 12}),
        );
        streamer.learn(14, &json!({"video_codec": "H264"}));
        let packet = |ssrc: u32, payload_type: u8| rtp::Packet {
            payload_type,
            marker: false,
            sequence: 0,
            timestamp: 0,
            ssrc,
            payload: &[],
        };
        assert_eq!(
            streamer.codec(&packet(11, rtp::VP8)),
            Some(Codec::Vp8),
            "by payload type"
        );
        assert_eq!(
            streamer.codec(&packet(11, 96)),
            Some(Codec::H264),
            "by Discord's choice"
        );
        assert_eq!(
            streamer.codec(&packet(12, rtp::VP8)),
            None,
            "retransmissions aren't frames"
        );
        assert_eq!(streamer.codec(&packet(30, 96)), None);
    }
}
