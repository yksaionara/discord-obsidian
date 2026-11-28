//! Plays a voice message, as Discord's client plays one: its Ogg file,
//! downloaded whole (a minute of Discord's Opus is about 250 KB), decoded on a
//! thread of its own as it plays, and played faster or slower without its
//! pitch changing, as a browser's `playbackRate` plays it. One plays at a
//! time; dropping it stops it.
//!
//! What it does shows through its [`VoiceClock`], which the window reads
//! while it plays rather than an event for each moment.

use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
#[cfg(feature = "discord")]
use std::{io::Cursor, sync::Arc, time::Duration};

#[cfg(feature = "discord")]
use anyhow::{Context, Result, bail};

#[cfg(feature = "discord")]
use crate::{audio_io::Output, video::Video, video_player::Resampler};

mod record;
#[cfg(feature = "discord")]
pub use record::Recorder;
pub use record::{MAX_MS, MIN_MS, Recorded, Recording, WARN_MS};

#[cfg(feature = "discord")]
const RATE: u32 = crate::audio_io::RATE;
/// The speeds Discord's client steps through, its first press from 1x.
pub const RATES: [f32; 4] = [0.75, 1.0, 1.5, 2.0];
/// The largest file played, well over Discord's 20 minutes.
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

/// How the voice message playing goes, shared with the thread playing it.
#[derive(Debug)]
pub struct VoiceClock {
    /// How far in it's heard, in milliseconds.
    position_ms: AtomicU64,
    /// How long it plays, once the file says; 0 before.
    duration_ms: AtomicU64,
    playing: AtomicBool,
    /// Still downloading or opening.
    loading: AtomicBool,
    /// Couldn't be played.
    failed: AtomicBool,
    /// A move asked for and not yet made: milliseconds, or [`NO_SEEK`].
    seek: AtomicU64,
    /// The speed, and how loud, as f32 bits.
    rate: AtomicU32,
    gain: AtomicU32,
    #[cfg_attr(not(feature = "discord"), allow(dead_code))]
    stop: AtomicBool,
    /// Wakes the thread while it's paused.
    changed: Condvar,
    lock: Mutex<()>,
}

const NO_SEEK: u64 = u64::MAX;

impl VoiceClock {
    /// A clock at `from_ms`, playing or paused.
    pub fn new(rate: f32, gain: f32, from_ms: u64, playing: bool) -> Self {
        Self {
            position_ms: AtomicU64::new(from_ms),
            duration_ms: AtomicU64::new(0),
            playing: AtomicBool::new(playing),
            loading: AtomicBool::new(true),
            failed: AtomicBool::new(false),
            seek: AtomicU64::new(if from_ms > 0 { from_ms } else { NO_SEEK }),
            rate: AtomicU32::new(rate.to_bits()),
            gain: AtomicU32::new(gain.to_bits()),
            stop: AtomicBool::new(false),
            changed: Condvar::new(),
            lock: Mutex::new(()),
        }
    }

    pub fn position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::Relaxed)
    }

    /// How long it plays, once known.
    pub fn duration_ms(&self) -> Option<u64> {
        Some(self.duration_ms.load(Ordering::Relaxed)).filter(|duration| *duration > 0)
    }

    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn loading(&self) -> bool {
        self.loading.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn rate(&self) -> f32 {
        f32::from_bits(self.rate.load(Ordering::Relaxed))
    }

    fn wake(&self) {
        let _guard = self.lock.lock();
        self.changed.notify_all();
    }

    pub fn play(&self, playing: bool) {
        self.playing.store(playing, Ordering::Relaxed);
        self.wake();
    }

    /// Moves to this many milliseconds in.
    pub fn seek(&self, to_ms: u64) {
        self.position_ms.store(to_ms, Ordering::Relaxed);
        self.seek.store(to_ms, Ordering::Relaxed);
        self.wake();
    }

    pub fn set_rate(&self, rate: f32) {
        self.rate
            .store(rate.clamp(0.5, 2.0).to_bits(), Ordering::Relaxed);
    }

    /// What its sound is multiplied by.
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }

    #[cfg(feature = "discord")]
    fn failed_with(&self, error: &anyhow::Error) {
        tracing::warn!(
            error = format!("{error:#}"),
            "could not play a voice message"
        );
        self.failed.store(true, Ordering::Relaxed);
        self.loading.store(false, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
    }
}

/// A voice message playing, until dropped.
#[cfg(feature = "discord")]
pub struct VoicePlayer {
    pub clock: Arc<VoiceClock>,
}

#[cfg(feature = "discord")]
impl VoicePlayer {
    /// Waits for `file`, then plays it on `speakers` as `clock` says.
    pub fn start(
        file: impl Future<Output = Result<Vec<u8>>> + Send + 'static,
        speakers: Option<String>,
        clock: Arc<VoiceClock>,
    ) -> Self {
        let playing = clock.clone();
        tokio::spawn(async move {
            let data = match file.await {
                Ok(data) => data,
                Err(error) => return playing.failed_with(&error),
            };
            if playing.stop.load(Ordering::Relaxed) {
                return;
            }
            let thread = playing.clone();
            // Opening the speakers and decoding block, so it has a thread.
            let spawned = std::thread::Builder::new()
                .name("obsidian-voice-message".into())
                .spawn(move || {
                    if let Err(error) = play(&data, speakers.as_deref(), &thread) {
                        thread.failed_with(&error);
                    }
                });
            if let Err(error) = spawned {
                playing.failed_with(&error.into());
            }
        });
        Self { clock }
    }
}

#[cfg(feature = "discord")]
impl Drop for VoicePlayer {
    fn drop(&mut self) {
        self.clock.stop.store(true, Ordering::Relaxed);
        self.clock.wake();
    }
}

/// Plays `data` until its end, or until it's stopped.
#[cfg(feature = "discord")]
fn play(data: &[u8], speakers: Option<&str>, clock: &VoiceClock) -> Result<()> {
    // Discord's voice messages are Ogg; a file that isn't one isn't
    // decoded, so a message claiming to be one can't reach other demuxers.
    if !data.starts_with(b"OggS") {
        bail!("a voice message that isn't an Ogg file");
    }
    let mut file =
        Video::open_sound(Cursor::new(data)).context("not a sound Discord Obsidian can play")?;
    let (rate, channels) = file.sound.context("the file has no sound")?;
    if let Some(duration) = file.duration_ms.filter(|duration| *duration > 0) {
        clock.duration_ms.store(duration, Ordering::Relaxed);
    }
    // About 100 ms queued, as a video's sound is.
    let mut output = Output::<f32>::open_or_default(
        speakers,
        "Voice message",
        RATE,
        2,
        Some(Duration::from_millis(100)),
    )
    .context("could not open the speakers")?;
    clock.loading.store(false, Ordering::Relaxed);
    let mut resampler = Resampler::new(rate, channels);
    let mut stretch = Stretch::default();
    // Where what's been written plays to, in input frames at RATE.
    let mut written = 0f64;
    let mut ended = false;
    let mut chunk = Vec::new();
    loop {
        if clock.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let seek = clock.seek.swap(NO_SEEK, Ordering::Relaxed);
        if seek != NO_SEEK {
            file.seek(seek)?;
            resampler = Resampler::new(rate, channels);
            stretch = Stretch::default();
            output.flush();
            written = seek as f64 * f64::from(RATE) / 1000.0;
            ended = false;
        }
        if !clock.playing() {
            output.flush();
            let guard = clock
                .lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !clock.playing() && !clock.stop.load(Ordering::Relaxed) {
                let _ = clock
                    .changed
                    .wait_timeout(guard, Duration::from_millis(250));
            }
            continue;
        }
        let speed = clock.rate();
        chunk.clear();
        let consumed = if ended {
            stretch.finish(speed, &mut chunk)
        } else if file.decode_sound()?.is_some() {
            let sound = file.sound();
            resampler.follow(sound.rate, sound.channels);
            let frames = resampler.run(sound.samples);
            if speed == 1.0 && stretch.is_empty() {
                chunk.extend_from_slice(&frames);
                frames.len() as f64 / 2.0
            } else {
                stretch.push(&frames);
                stretch.run(speed, &mut chunk)
            }
        } else {
            ended = true;
            continue;
        };
        if ended && chunk.is_empty() {
            // Heard to the end: it waits at the start, as Discord's does.
            let _ = output.drain();
            if clock.seek.load(Ordering::Relaxed) == NO_SEEK {
                clock.playing.store(false, Ordering::Relaxed);
                clock.position_ms.store(0, Ordering::Relaxed);
                clock.seek.store(0, Ordering::Relaxed);
            }
            continue;
        }
        let gain = f32::from_bits(clock.gain.load(Ordering::Relaxed));
        if gain != 1.0 {
            chunk.iter_mut().for_each(|sample| *sample *= gain);
        }
        output.write(&chunk)?;
        written += consumed;
        // What plays now was written a little before.
        let behind = output.latency().as_secs_f64() * f64::from(RATE) * f64::from(speed);
        let heard = ((written - behind).max(0.0) * 1000.0 / f64::from(RATE)) as u64;
        if clock.seek.load(Ordering::Relaxed) == NO_SEEK {
            clock.position_ms.store(heard, Ordering::Relaxed);
        }
    }
}

/// How long a piece of sound the stretch overlaps, its step, and how far
/// either way it looks for the piece that best continues the last: 40, 20
/// and 10 ms. Speech keeps its pitch, as WSOLA keeps it.
#[cfg(feature = "discord")]
const SEGMENT: usize = 1920;
#[cfg(feature = "discord")]
const HOP: usize = SEGMENT / 2;
#[cfg(feature = "discord")]
const SEEK: usize = 480;

/// Plays stereo sound faster or slower without changing its pitch, by
/// overlapping pieces of it taken at the speed's pace (WSOLA).
#[cfg(feature = "discord")]
#[derive(Default)]
struct Stretch {
    /// Frames not yet passed by, from `input[0]`.
    input: Vec<[f32; 2]>,
    /// Where the next piece is wanted, in frames of `input`.
    next: f64,
    /// Where the last piece started, in frames of `input`.
    last: Option<usize>,
    /// The last piece's second half, faded out, which the next fades in over.
    tail: Vec<[f32; 2]>,
}

#[cfg(feature = "discord")]
impl Stretch {
    fn is_empty(&self) -> bool {
        self.input.is_empty() && self.last.is_none()
    }

    fn push(&mut self, interleaved: &[f32]) {
        self.input.extend(
            interleaved
                .chunks_exact(2)
                .map(|frame| [frame[0], frame[1]]),
        );
    }

    /// Plays out what's left: the input with silence after it.
    fn finish(&mut self, speed: f32, out: &mut Vec<f32>) -> f64 {
        if self.input.is_empty() && self.tail.is_empty() {
            return 0.0;
        }
        let real = self.input.len();
        self.input.resize(real + SEGMENT + 2 * SEEK, [0.0; 2]);
        let consumed = self.run(speed, out);
        out.extend(self.tail.drain(..).flatten());
        self.input.clear();
        self.last = None;
        self.next = 0.0;
        consumed.min(real as f64)
    }

    /// Overlaps what's ready into `out`; returns how much input it played.
    fn run(&mut self, speed: f32, out: &mut Vec<f32>) -> f64 {
        let step = HOP as f64 * f64::from(speed);
        let mut consumed = 0.0;
        loop {
            let target = self.next as usize;
            if target + SEEK + SEGMENT > self.input.len() {
                break;
            }
            let start = match self.last {
                None => target,
                Some(last) => self.best(last + HOP, target),
            };
            if self.tail.is_empty() {
                self.tail.resize(HOP, [0.0; 2]);
            }
            for k in 0..HOP {
                let weight = window(k);
                let [left, right] = self.input[start + k];
                out.push(self.tail[k][0] + left * weight);
                out.push(self.tail[k][1] + right * weight);
            }
            for k in 0..HOP {
                let weight = window(HOP + k);
                let [left, right] = self.input[start + HOP + k];
                self.tail[k] = [left * weight, right * weight];
            }
            self.last = Some(start);
            self.next += step;
            consumed += step;
            // Keep what the next search and continuation can reach.
            let keep = (self.next as usize).saturating_sub(SEEK).min(start + HOP);
            if keep > 0 {
                self.input.drain(..keep);
                self.next -= keep as f64;
                self.last = Some(start - keep);
            }
        }
        consumed
    }

    /// Where near `target` a piece best continues the sound at `natural`.
    fn best(&self, natural: usize, target: usize) -> usize {
        let mono = |index: usize| self.input[index][0] + self.input[index][1];
        let low = target.saturating_sub(SEEK);
        let high = target + SEEK;
        let mut best = (f32::MIN, target);
        // Every other place, every fourth frame: close enough for speech.
        for candidate in (low..=high).step_by(2) {
            let mut score = 0.0;
            for k in (0..HOP).step_by(4) {
                score += mono(natural + k) * mono(candidate + k);
            }
            if score > best.0 {
                best = (score, candidate);
            }
        }
        best.1
    }
}

/// A periodic Hann window over a piece: overlapped by half, they sum to 1.
#[cfg(feature = "discord")]
fn window(k: usize) -> f32 {
    0.5 - 0.5 * (std::f32::consts::TAU * k as f32 / SEGMENT as f32).cos()
}

/// The speed after `rate`, as Discord's button steps: 0.75, 1, 1.5, 2.
pub fn next_rate(rate: f32) -> f32 {
    let at = RATES.iter().position(|known| *known == rate).unwrap_or(1);
    RATES[(at + 1) % RATES.len()]
}

/// How Discord's button names a speed: "1X", ".75X", "1.5X".
pub fn rate_label(rate: f32) -> String {
    let text = format!("{rate}");
    format!("{}X", text.strip_prefix('0').unwrap_or(&text))
}

/// A waveform as `bars` levels from 0 to 1, as Discord's client draws it:
/// averaged down when it has more, stretched with straight lines between
/// its levels when it has fewer.
pub fn bars(waveform: &[u8], bars: usize) -> Vec<f32> {
    let levels: Vec<f32> = waveform
        .iter()
        .map(|level| f32::from(*level) / 255.0)
        .collect();
    match levels.len() {
        _ if bars == 0 => Vec::new(),
        0 => vec![0.0; bars],
        1 => vec![levels[0]; bars],
        length if length < bars => (0..bars)
            .map(|index| {
                let at = index as f32 * (length - 1) as f32 / (bars - 1).max(1) as f32;
                let low = at as usize;
                let high = (low + 1).min(length - 1);
                let fraction = at - low as f32;
                levels[low] * (1.0 - fraction) + levels[high] * fraction
            })
            .collect(),
        length => {
            let per = length as f64 / bars as f64;
            let mut out = Vec::with_capacity(bars);
            let mut from = 0;
            while out.len() < bars {
                let to = (((out.len() + 1) as f64 * per).round() as usize).min(length);
                let slice = &levels[from..to.max(from + 1).min(length)];
                out.push(slice.iter().sum::<f32>() / slice.len() as f32);
                from = to;
            }
            out
        }
    }
}

/// How many bars Discord's client draws for a voice message this long: its
/// waveform is 40 px wide for half a second or less, 294 px from 45 s, and in
/// between in proportion, a bar every 6 px.
pub fn bar_count(duration_ms: u64) -> usize {
    let seconds = duration_ms as f64 / 1000.0;
    let width = if seconds <= 0.5 {
        40.0
    } else if seconds >= 45.0 {
        294.0
    } else {
        (seconds - 0.5) / 44.5 * 254.0 + 40.0
    };
    let width = 6.0 * ((width + 4.0) / 6.0).floor() - 4.0;
    ((width + 4.0) / 6.0).floor() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_through_discords_speeds() {
        assert_eq!(next_rate(1.0), 1.5);
        assert_eq!(next_rate(1.5), 2.0);
        assert_eq!(next_rate(2.0), 0.75);
        assert_eq!(next_rate(0.75), 1.0);
        assert_eq!(rate_label(0.75), ".75X");
        assert_eq!(rate_label(1.0), "1X");
        assert_eq!(rate_label(1.5), "1.5X");
    }

    #[test]
    fn draws_as_many_bars_as_discord() {
        assert_eq!(bar_count(300), 7, "40 px");
        assert_eq!(bar_count(60_000), 49, "294 px");
        assert_eq!(bar_count(10_000), 16);
        assert_eq!(bars(&[0, 255], 3), [0.0, 0.5, 1.0]);
        assert_eq!(bars(&[255, 255, 0, 0], 2), [1.0, 0.0]);
        assert_eq!(bars(&[], 2), [0.0, 0.0]);
    }

    #[cfg(feature = "discord")]
    #[test]
    fn a_faster_speed_plays_the_same_sound_in_less_time() {
        let mut stretch = Stretch::default();
        // A second of a 200 Hz tone, stereo.
        let tone: Vec<f32> = (0..RATE as usize)
            .flat_map(|index| {
                let sample = (index as f32 * std::f32::consts::TAU * 200.0 / RATE as f32).sin();
                [sample, sample]
            })
            .collect();
        let mut out = Vec::new();
        stretch.push(&tone);
        let mut consumed = stretch.run(2.0, &mut out);
        consumed += stretch.finish(2.0, &mut out);
        let frames = out.len() / 2;
        assert!(
            (consumed - f64::from(RATE)).abs() < 2.0 * HOP as f64,
            "{consumed}"
        );
        assert!(
            (frames as i64 - (RATE / 2) as i64).abs() < 2 * SEGMENT as i64,
            "half as long: {frames}"
        );
        // The pitch stays: as many rises through zero per second of output
        // as the tone has, about 200.
        let middle = &out[SEGMENT * 2..out.len() - SEGMENT * 2];
        let rises = middle
            .chunks_exact(2)
            .map(|frame| frame[0])
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count();
        let seconds = middle.len() as f32 / 2.0 / RATE as f32;
        let pitch = rises as f32 / seconds;
        assert!((pitch - 200.0).abs() < 15.0, "{pitch} Hz");
    }
}
