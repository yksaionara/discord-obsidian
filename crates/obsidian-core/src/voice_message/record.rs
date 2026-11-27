//! Records a voice message, as Discord's client records one: the
//! microphone in mono at 48 kHz, as Opus in an Ogg file, written to disk as
//! it's made, with how loud it is along the way for its waveform. It stops
//! by itself at Discord's longest, 1199.5 seconds, and one shorter than half
//! a second isn't kept.
//!
//! What it does shows through its [`Recording`], which the window reads
//! while it records rather than an event for each moment.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
};

/// The longest voice message Discord's client records, in milliseconds.
pub const MAX_MS: u64 = 1_199_500;
/// The shortest it keeps.
pub const MIN_MS: u64 = 500;
/// When the timer starts to blink, as Discord's does near the end.
pub const WARN_MS: u64 = 1_169_500;
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
/// The most levels Discord keeps for a waveform.
pub const WAVEFORM_LEVELS: usize = 256;
/// A level is kept every 100 ms, as the bar shows them while recording.
#[cfg(feature = "discord")]
const LEVEL_MS: u64 = 100;

const RECORDING: u8 = 0;
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
const STOPPED: u8 = 1;
const FAILED: u8 = 2;

/// A recording finished and on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub path: PathBuf,
    pub duration_ms: u32,
    /// Up to [`WAVEFORM_LEVELS`] levels from 0 to 255, as Discord sends.
    pub waveform: Arc<[u8]>,
}

/// How a recording goes, shared with the thread making it.
#[derive(Debug, Default)]
pub struct Recording {
    /// How long it's recorded, in milliseconds.
    elapsed_ms: AtomicU64,
    state: AtomicU8,
    stop: AtomicBool,
    /// Thrown away: the file goes once the thread is done with it.
    #[cfg_attr(not(feature = "discord"), allow(dead_code))]
    discard: AtomicBool,
    /// How loud each 100 ms was, from 0 to 255.
    levels: Mutex<Vec<u8>>,
    finished: Mutex<Option<Recorded>>,
}

impl Recording {
    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms.load(Ordering::Relaxed)
    }

    /// Still listening.
    pub fn recording(&self) -> bool {
        self.state.load(Ordering::Relaxed) == RECORDING
    }

    /// The microphone wouldn't open.
    pub fn failed(&self) -> bool {
        self.state.load(Ordering::Relaxed) == FAILED
    }

    /// The finished recording, once it's stopped.
    pub fn finished(&self) -> Option<Recorded> {
        lock(&self.finished).clone()
    }

    /// The last `count` levels, from 0 to 1, oldest first, as the bar
    /// draws them while recording; silence before the start.
    pub fn recent(&self, count: usize) -> Vec<f32> {
        let levels = lock(&self.levels);
        let shown = &levels[levels.len().saturating_sub(count)..];
        std::iter::repeat_n(0.0, count - shown.len())
            .chain(shown.iter().map(|level| f32::from(*level) / 255.0))
            .collect()
    }

    /// Stops listening; the thread finishes the file.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Stopped, with this recording, as its thread leaves it.
    #[cfg(all(test, feature = "discord"))]
    pub(crate) fn finish_with(&self, recorded: Recorded) {
        *lock(&self.finished) = Some(recorded);
        self.state.store(STOPPED, Ordering::Relaxed);
    }

    #[cfg_attr(not(feature = "discord"), allow(dead_code))]
    fn level(&self, level: u8) {
        lock(&self.levels).push(level);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg_attr(not(feature = "discord"), allow(dead_code))]
/// A level from 0 to 255 for a stretch of sound's loudness: its decibels
/// from -100 to 0, as Discord's client maps them.
pub fn level(rms: f32) -> u8 {
    let decibels = 20.0 * rms.max(1e-5).log10();
    ((decibels + 100.0) / 100.0 * 255.0)
        .clamp(0.0, 255.0)
        .round() as u8
}

#[cfg_attr(not(feature = "discord"), allow(dead_code))]
/// Levels averaged down to at most [`WAVEFORM_LEVELS`], as Discord sends a
/// waveform.
pub fn waveform(levels: &[u8]) -> Arc<[u8]> {
    if levels.len() <= WAVEFORM_LEVELS {
        return levels.into();
    }
    (0..WAVEFORM_LEVELS)
        .map(|bucket| {
            let start = bucket * levels.len() / WAVEFORM_LEVELS;
            let end = ((bucket + 1) * levels.len() / WAVEFORM_LEVELS).max(start + 1);
            let part = &levels[start..end];
            (part.iter().map(|level| u32::from(*level)).sum::<u32>() / part.len() as u32) as u8
        })
        .collect()
}

#[cfg(feature = "discord")]
pub use recorder::Recorder;

#[cfg(feature = "discord")]
mod recorder {
    use std::{
        fs::File,
        io::{BufWriter, Write},
        path::{Path, PathBuf},
        sync::{Arc, atomic::Ordering},
        thread::JoinHandle,
        time::Duration,
    };

    use anyhow::{Context, Result, bail};
    use obsidian_discord::opus::{Application, Encoder};

    use super::{FAILED, LEVEL_MS, MAX_MS, MIN_MS, Recorded, Recording, STOPPED, lock, waveform};
    use crate::audio_io::Input;

    const RATE: u32 = 48_000;
    /// 20 ms frames, as Discord's client encodes them.
    const FRAME: usize = RATE as usize / 50;
    /// Speech at 32 kb/s in mono, about 240 KB a minute.
    const BITRATE: i32 = 32_000;

    /// A voice message recording, until it's finished or dropped; dropping
    /// it throws the recording away.
    pub struct Recorder {
        recording: Arc<Recording>,
        thread: Option<JoinHandle<Result<Recorded>>>,
    }

    impl Recorder {
        /// Records `microphone` into a new file at `path`, as `recording`
        /// shows. `failed` hears once if the microphone won't open.
        pub fn start(
            microphone: Option<String>,
            path: PathBuf,
            recording: Arc<Recording>,
            failed: impl FnOnce() + Send + 'static,
        ) -> Self {
            let shared = recording.clone();
            let thread = std::thread::Builder::new()
                .name("obsidian-voice-recording".into())
                .spawn(move || {
                    let result = record(microphone.as_deref(), &path, &shared);
                    match &result {
                        Ok(recorded) if shared.discard.load(Ordering::Relaxed) => {
                            let _ = std::fs::remove_file(&recorded.path);
                        }
                        Ok(recorded) => *lock(&shared.finished) = Some(recorded.clone()),
                        Err(error) => {
                            tracing::warn!(
                                error = format!("{error:#}"),
                                "could not record a voice message"
                            );
                            let _ = std::fs::remove_file(&path);
                            let started = shared.elapsed_ms() > 0;
                            shared.state.store(FAILED, Ordering::Relaxed);
                            if !started {
                                failed();
                            }
                            return result;
                        }
                    }
                    shared.state.store(STOPPED, Ordering::Relaxed);
                    result
                });
            if thread.is_err() {
                recording.state.store(FAILED, Ordering::Relaxed);
            }
            Self {
                recording,
                thread: thread.ok(),
            }
        }

        pub fn recording(&self) -> &Arc<Recording> {
            &self.recording
        }

        /// Stops, if it hasn't, and waits for the file.
        pub fn finish(mut self) -> Result<Recorded> {
            self.recording.stop();
            let thread = self.thread.take().context("the recording didn't start")?;
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("the recording stopped unexpectedly"))?
        }
    }

    impl Drop for Recorder {
        fn drop(&mut self) {
            let Some(thread) = self.thread.take() else {
                return;
            };
            self.recording.discard.store(true, Ordering::Relaxed);
            self.recording.stop();
            // It stops within a frame; waiting makes sure the file goes.
            if let Ok(Ok(recorded)) = thread.join() {
                let _ = std::fs::remove_file(recorded.path);
            }
        }
    }

    fn record(microphone: Option<&str>, path: &Path, recording: &Recording) -> Result<Recorded> {
        let mut input = Input::<f32>::open(
            microphone,
            "Voice message",
            RATE,
            1,
            Duration::from_millis(20),
        )
        .context("could not open the microphone")?;
        let mut encoder = Encoder::new(1, Application::Recording, BITRATE)?;
        let pre_skip = encoder.lookahead();
        let file = File::create(path).context("could not make the recording's file")?;
        let mut ogg = Ogg::new(BufWriter::new(file), rand::random());
        ogg.header(&opus_head(pre_skip))?;
        ogg.header(&opus_tags())?;
        let mut frame = vec![0f32; FRAME];
        let mut packet = vec![0u8; 1500];
        let mut levels = Vec::new();
        let mut frames = 0u64;
        // The loudest 20 ms of the 100 ms being measured.
        let mut loudest = 0f32;
        loop {
            match input.read(&mut frame) {
                Ok(()) => {}
                // What was heard before the microphone went is kept.
                Err(error) if frames > 0 => {
                    tracing::warn!(%error, "the microphone stopped while recording");
                    break;
                }
                Err(error) => return Err(error.context("the microphone stopped")),
            }
            let length = encoder.encode_float(&frame, &mut packet)?;
            frames += 1;
            ogg.packet(
                &packet[..length],
                frames * FRAME as u64 + u64::from(pre_skip),
            )?;
            let power = frame.iter().map(|sample| sample * sample).sum::<f32>() / FRAME as f32;
            loudest = loudest.max(power.sqrt());
            let elapsed = frames * 20;
            if elapsed.is_multiple_of(LEVEL_MS) {
                let level = super::level(loudest);
                levels.push(level);
                recording.level(level);
                loudest = 0.0;
            }
            recording.elapsed_ms.store(elapsed, Ordering::Relaxed);
            if recording.stop.load(Ordering::Relaxed) || elapsed >= MAX_MS {
                break;
            }
        }
        ogg.finish()?;
        let duration_ms = frames * 20;
        if duration_ms < MIN_MS {
            let _ = std::fs::remove_file(path);
            bail!("too short to keep");
        }
        Ok(Recorded {
            path: path.to_owned(),
            duration_ms: duration_ms as u32,
            waveform: waveform(&levels),
        })
    }

    /// Opus's identification header for one channel at 48 kHz (RFC 7845).
    fn opus_head(pre_skip: u16) -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.push(1);
        head.push(1);
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&RATE.to_le_bytes());
        head.extend_from_slice(&0i16.to_le_bytes());
        head.push(0);
        head
    }

    /// Opus's comment header: the encoder, and no comments.
    fn opus_tags() -> Vec<u8> {
        let vendor = b"libopus";
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        tags
    }

    /// Packets into Ogg pages (RFC 3533), each written as it fills: a
    /// second of sound or 255 segments, whichever comes first.
    pub(super) struct Ogg<W: Write> {
        out: W,
        serial: u32,
        sequence: u32,
        /// The page being filled: its segment table, data and the granule
        /// of its last packet.
        segments: Vec<u8>,
        data: Vec<u8>,
        granule: u64,
        packets: usize,
        started: bool,
    }

    const PAGE_PACKETS: usize = 50;

    impl<W: Write> Ogg<W> {
        pub(super) fn new(out: W, serial: u32) -> Self {
            Self {
                out,
                serial,
                sequence: 0,
                segments: Vec::with_capacity(255),
                data: Vec::new(),
                granule: 0,
                packets: 0,
                started: false,
            }
        }

        /// A header packet, on a page of its own.
        pub(super) fn header(&mut self, packet: &[u8]) -> Result<()> {
            self.add(packet, 0);
            self.page(false)
        }

        /// A packet that ends at `granule` samples.
        pub(super) fn packet(&mut self, packet: &[u8], granule: u64) -> Result<()> {
            let segments = packet.len() / 255 + 1;
            if self.packets >= PAGE_PACKETS || self.segments.len() + segments > 255 {
                self.page(false)?;
            }
            self.add(packet, granule);
            Ok(())
        }

        /// The last page, marked as the end of the stream.
        pub(super) fn finish(mut self) -> Result<W> {
            self.page(true)?;
            self.out.flush()?;
            Ok(self.out)
        }

        fn add(&mut self, packet: &[u8], granule: u64) {
            self.segments
                .extend(std::iter::repeat_n(255, packet.len() / 255));
            self.segments.push((packet.len() % 255) as u8);
            self.data.extend_from_slice(packet);
            self.granule = granule;
            self.packets += 1;
        }

        fn page(&mut self, last: bool) -> Result<()> {
            let mut kind = 0u8;
            if !self.started {
                kind |= 0x02;
            }
            if last {
                kind |= 0x04;
            }
            let mut page = Vec::with_capacity(27 + self.segments.len() + self.data.len());
            page.extend_from_slice(b"OggS");
            page.push(0);
            page.push(kind);
            page.extend_from_slice(&self.granule.to_le_bytes());
            page.extend_from_slice(&self.serial.to_le_bytes());
            page.extend_from_slice(&self.sequence.to_le_bytes());
            page.extend_from_slice(&[0; 4]);
            page.push(self.segments.len() as u8);
            page.extend_from_slice(&self.segments);
            page.extend_from_slice(&self.data);
            let crc = crc(&page);
            page[22..26].copy_from_slice(&crc.to_le_bytes());
            self.out.write_all(&page)?;
            self.started = true;
            self.sequence += 1;
            self.segments.clear();
            self.data.clear();
            self.packets = 0;
            Ok(())
        }
    }

    /// Ogg's checksum: CRC-32 with polynomial 0x04c11db7, unreflected, from 0.
    pub(super) fn crc(data: &[u8]) -> u32 {
        data.iter().fold(0u32, |crc, byte| {
            let mut crc = crc ^ (u32::from(*byte) << 24);
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04c1_1db7
                } else {
                    crc << 1
                };
            }
            crc
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The first page of an Ogg file libogg wrote checks out.
        #[test]
        fn checksums_as_ogg_does() {
            let file = std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../testing/fixtures/voice-tone.ogg"
            ))
            .unwrap();
            let length = 27 + usize::from(file[26]);
            let length = length
                + file[27..length]
                    .iter()
                    .map(|segment| usize::from(*segment))
                    .sum::<usize>();
            let mut page = file[..length].to_vec();
            let written = u32::from_le_bytes(page[22..26].try_into().unwrap());
            page[22..26].fill(0);
            assert_eq!(crc(&page), written);
        }

        /// Two seconds of a tone, through the encoder and into pages, opens and
        /// plays as the voice message player opens Discord's.
        #[test]
        fn a_recording_is_a_file_the_player_opens() {
            let mut encoder = Encoder::new(1, Application::Recording, BITRATE).unwrap();
            let pre_skip = encoder.lookahead();
            let mut ogg = Ogg::new(Vec::new(), 7);
            ogg.header(&opus_head(pre_skip)).unwrap();
            ogg.header(&opus_tags()).unwrap();
            let mut packet = vec![0u8; 1500];
            for index in 0..100u64 {
                let frame: Vec<f32> = (0..FRAME)
                    .map(|sample| {
                        let at = index as f32 * FRAME as f32 + sample as f32;
                        0.5 * (at * std::f32::consts::TAU * 440.0 / RATE as f32).sin()
                    })
                    .collect();
                let length = encoder.encode_float(&frame, &mut packet).unwrap();
                ogg.packet(
                    &packet[..length],
                    (index + 1) * FRAME as u64 + u64::from(pre_skip),
                )
                .unwrap();
            }
            let file = ogg.finish().unwrap();
            assert!(file.starts_with(b"OggS"));
            let mut sound = crate::video::Video::open_sound(std::io::Cursor::new(file)).unwrap();
            let (rate, channels) = sound.sound.unwrap();
            assert_eq!(channels, 1);
            assert!(rate > 0);
            if let Some(duration) = sound.duration_ms {
                assert!((1950..=2050).contains(&duration), "{duration} ms");
            }
            let mut samples = 0;
            while sound.decode_sound().unwrap().is_some() {
                samples += sound.sound().samples.len();
            }
            assert!(samples > 90_000, "{samples} samples");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_span_a_hundred_decibels() {
        assert_eq!(level(1.0), 255);
        assert_eq!(level(0.0), 0);
        assert_eq!(level(0.001), 102, "-60 dB");
    }

    #[test]
    fn a_waveform_is_averaged_down_to_256_levels() {
        assert_eq!(&*waveform(&[1, 2, 3]), [1, 2, 3]);
        let long: Vec<u8> = (0..512).map(|index| (index % 2 * 200) as u8).collect();
        let short = waveform(&long);
        assert_eq!(short.len(), 256);
        assert!(short.iter().all(|level| *level == 100));
    }

    #[test]
    fn the_bar_shows_the_latest_levels_after_silence() {
        let recording = Recording::default();
        recording.level(255);
        recording.level(0);
        assert_eq!(recording.recent(3), [0.0, 1.0, 0.0]);
        assert!(recording.recording());
        assert_eq!(recording.finished(), None);
    }
}
