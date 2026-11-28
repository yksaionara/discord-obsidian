//! A stream's delay-based congestion control, as Discord's client runs it:
//! libwebrtc's send-side estimate (GCC, draft-ietf-rmcat-gcc), from the
//! transport-wide congestion control feedback Discord's server sends about
//! five to ten times a second (RTCP 205, format 15).
//!
//! Each video packet carries a transport-wide sequence number (extension
//! 5). The feedback says which arrived and when, by the server's clock.
//! Packets sent within 5 ms of each other make a group; for consecutive
//! groups, how much further apart they arrived than they were sent is the
//! queueing delay they gained. A trendline over the last 20 groups'
//! accumulated delay, against an adaptive threshold, says whether a queue
//! is building (overuse), draining (underuse), or neither. The rate then
//! drops to 85% of what the server acknowledged receiving on overuse, holds
//! on underuse, and otherwise grows: 8% a second, or 2% within 10% of the
//! capacity the last overuse showed, and never past one and a half times
//! what the server acknowledged. So the stream backs off as a queue
//! starts to build, before packets are lost, where a loss-based estimate
//! only learns of a full path from the loss.
//!
//! Unlike Discord's client, Discord Obsidian doesn't also cap the rate at
//! the server's own estimate (REMB): that held 1440p streams at 1.5 Mbit/s
//! on paths that carried far more.

use std::collections::VecDeque;

/// The transport-wide feedback in a decrypted compound RTCP packet: each
/// packet it reports on, by transport sequence number, with when it
/// arrived in microseconds of the server's clock, or None for one that
/// hasn't.
pub fn transport_feedback(mut rtcp: &[u8]) -> Vec<Vec<(u16, Option<i64>)>> {
    let mut reports = vec![];
    while rtcp.len() >= 4 {
        let length = (usize::from(u16::from_be_bytes([rtcp[2], rtcp[3]])) + 1) * 4;
        let packet = &rtcp[..length.min(rtcp.len())];
        if packet[1] == 205
            && packet[0] & 0x1f == 15
            && let Some(report) = parse_feedback(packet)
        {
            reports.push(report);
        }
        rtcp = rtcp.get(length..).unwrap_or_default();
    }
    reports
}

/// One feedback packet: its header and two SSRCs, the base sequence number,
/// the count of packets, a 24-bit reference time in 64 ms, a feedback
/// count, then status chunks and, for each packet received, its arrival
/// delta in 250 µs.
fn parse_feedback(packet: &[u8]) -> Option<Vec<(u16, Option<i64>)>> {
    let byte = |at: usize| packet.get(at).copied();
    let word = |at: usize| Some(u16::from_be_bytes([byte(at)?, byte(at + 1)?]));
    let base = word(12)?;
    let count = usize::from(word(14)?);
    let reference = i32::from_be_bytes([0, byte(16)?, byte(17)?, byte(18)?]);
    // 24 bits, signed.
    let reference = if reference & 0x80_0000 != 0 {
        reference - 0x100_0000
    } else {
        reference
    };
    let mut at = 20;
    let mut symbols = Vec::with_capacity(count);
    while symbols.len() < count {
        let chunk = word(at)?;
        at += 2;
        if chunk & 0x8000 == 0 {
            // A run of one symbol.
            let symbol = ((chunk >> 13) & 3) as u8;
            let run = usize::from(chunk & 0x1fff).min(count - symbols.len());
            if run == 0 {
                return None;
            }
            symbols.extend(std::iter::repeat_n(symbol, run));
        } else if chunk & 0x4000 == 0 {
            // Fourteen one-bit symbols: received with a small delta, or not.
            for bit in (0..14).rev() {
                if symbols.len() < count {
                    symbols.push(((chunk >> bit) & 1) as u8);
                }
            }
        } else {
            // Seven two-bit symbols.
            for pair in (0..7).rev() {
                if symbols.len() < count {
                    symbols.push(((chunk >> (pair * 2)) & 3) as u8);
                }
            }
        }
    }
    let mut time = i64::from(reference) * 64_000;
    let mut report = Vec::with_capacity(count);
    for (index, symbol) in symbols.into_iter().enumerate() {
        let sequence = base.wrapping_add(index as u16);
        match symbol {
            1 => {
                time += i64::from(byte(at)?) * 250;
                at += 1;
                report.push((sequence, Some(time)));
            }
            2 => {
                time += i64::from(i16::from_be_bytes([byte(at)?, byte(at + 1)?])) * 250;
                at += 2;
                report.push((sequence, Some(time)));
            }
            _ => report.push((sequence, None)),
        }
    }
    Some(report)
}

/// When each packet lately sent went, by transport sequence number, and its
/// size, for matching feedback to: about a second and a half at 30 Mbit/s.
pub struct SendHistory {
    slots: Vec<Option<(u16, i64, u32)>>,
}

impl Default for SendHistory {
    fn default() -> Self {
        Self {
            slots: vec![None; 4096],
        }
    }
}

impl SendHistory {
    /// Notes that the packet `sequence` of `size` bytes went at `sent`, in
    /// microseconds of this side's clock.
    pub fn record(&mut self, sequence: u16, sent: i64, size: u32) {
        let slot = usize::from(sequence) % self.slots.len();
        self.slots[slot] = Some((sequence, sent, size));
    }

    /// The packets `report` says arrived, as (sent, arrived, size), in the
    /// order they were sent; those not remembered are left out.
    pub fn arrivals(&self, report: &[(u16, Option<i64>)]) -> Vec<(i64, i64, u32)> {
        let mut arrived: Vec<(i64, i64, u32)> = report
            .iter()
            .filter_map(|&(sequence, arrival)| {
                let arrival = arrival?;
                match self.slots[usize::from(sequence) % self.slots.len()] {
                    Some((kept, sent, size)) if kept == sequence => Some((sent, arrival, size)),
                    _ => None,
                }
            })
            .collect();
        arrived.sort_by_key(|&(sent, ..)| sent);
        arrived
    }
}

/// What the delay says about the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Normal,
    /// A queue is building: send less.
    Overuse,
    /// A queue is draining: hold, so it empties.
    Underuse,
}

/// Packets sent within this of a group's first belong to it, as a frame's
/// do.
const GROUP_SPAN_US: i64 = 5_000;
/// How many groups the trendline's slope is fitted over.
const TREND_WINDOW: usize = 20;
const SMOOTHING: f64 = 0.9;
const THRESHOLD_GAIN: f64 = 4.0;
/// How long the trend must stay over the threshold before it counts.
const OVERUSE_TIME_MS: f64 = 10.0;
/// How fast the threshold follows the trend, up and down, per ms.
const K_UP: f64 = 0.0087;
const K_DOWN: f64 = 0.039;
/// How much the rate drops to on overuse, of what was acknowledged.
const BETA: f64 = 0.85;
/// The window acknowledged bitrate is measured over.
const ACKED_WINDOW_US: i64 = 500_000;

#[derive(Clone, Copy)]
struct Group {
    first_sent: i64,
    last_sent: i64,
    last_arrival: i64,
}

/// libwebrtc's trendline estimator and overuse detector.
struct Trendline {
    accumulated: f64,
    smoothed: f64,
    first_arrival: Option<f64>,
    window: VecDeque<(f64, f64)>,
    deltas: u32,
    threshold: f64,
    previous_trend: f64,
    /// Time over the threshold, ms; negative before it's crossed.
    over: f64,
    overuses: u32,
    last_update: Option<f64>,
    signal: Signal,
}

impl Default for Trendline {
    fn default() -> Self {
        Self {
            accumulated: 0.0,
            smoothed: 0.0,
            first_arrival: None,
            window: VecDeque::with_capacity(TREND_WINDOW + 1),
            deltas: 0,
            threshold: 12.5,
            previous_trend: 0.0,
            over: -1.0,
            overuses: 0,
            last_update: None,
            signal: Signal::Normal,
        }
    }
}

impl Trendline {
    /// Takes one group's arrival and send spacing against the group before,
    /// in ms, and when it arrived.
    fn update(&mut self, arrival_delta: f64, send_delta: f64, arrival: f64) -> Signal {
        let delay = arrival_delta - send_delta;
        self.deltas = (self.deltas + 1).min(1000);
        self.accumulated += delay;
        self.smoothed = SMOOTHING * self.smoothed + (1.0 - SMOOTHING) * self.accumulated;
        let origin = *self.first_arrival.get_or_insert(arrival);
        self.window.push_back((arrival - origin, self.smoothed));
        if self.window.len() > TREND_WINDOW {
            self.window.pop_front();
        }
        let trend = if self.window.len() == TREND_WINDOW {
            slope(&self.window).unwrap_or(self.previous_trend)
        } else {
            self.previous_trend
        };
        self.detect(trend, send_delta, arrival);
        self.signal
    }

    fn detect(&mut self, trend: f64, send_delta: f64, now: f64) {
        let modified = f64::from(self.deltas.min(60)) * trend * THRESHOLD_GAIN;
        if modified > self.threshold {
            if self.over < 0.0 {
                self.over = send_delta / 2.0;
            } else {
                self.over += send_delta;
            }
            self.overuses += 1;
            if self.over > OVERUSE_TIME_MS && self.overuses > 1 && trend >= self.previous_trend {
                self.over = 0.0;
                self.overuses = 0;
                self.signal = Signal::Overuse;
            }
        } else if modified < -self.threshold {
            self.over = -1.0;
            self.overuses = 0;
            self.signal = Signal::Underuse;
        } else {
            self.over = -1.0;
            self.overuses = 0;
            self.signal = Signal::Normal;
        }
        self.previous_trend = trend;
        // The threshold follows the trend, slowly up and faster down,
        // except for a jump far past it, as a sudden route change.
        let last = self.last_update.replace(now).unwrap_or(now);
        if modified.abs() <= self.threshold + 15.0 {
            let k = if modified.abs() < self.threshold {
                K_DOWN
            } else {
                K_UP
            };
            let elapsed = (now - last).clamp(0.0, 100.0);
            self.threshold = (self.threshold + k * (modified.abs() - self.threshold) * elapsed)
                .clamp(6.0, 600.0);
        }
    }
}

/// The least-squares slope of `points`.
fn slope(points: &VecDeque<(f64, f64)>) -> Option<f64> {
    let count = points.len() as f64;
    let mean_x = points.iter().map(|point| point.0).sum::<f64>() / count;
    let mean_y = points.iter().map(|point| point.1).sum::<f64>() / count;
    let (mut numerator, mut denominator) = (0.0, 0.0);
    for &(x, y) in points {
        numerator += (x - mean_x) * (y - mean_y);
        denominator += (x - mean_x) * (x - mean_x);
    }
    (denominator != 0.0).then(|| numerator / denominator)
}

/// The delay-based estimate: what to send, given the feedback so far.
pub struct DelayBased {
    bitrate: f64,
    trend: Trendline,
    current: Option<Group>,
    previous: Option<Group>,
    /// Packets acknowledged lately: when they arrived and their size.
    acked: VecDeque<(i64, u32)>,
    /// The acknowledged bitrate the last overuse came at, averaged.
    capacity: Option<f64>,
    /// When the rate last changed, in this side's microseconds; and whether
    /// it holds, after an overuse or while a queue drains.
    updated: Option<i64>,
    holding: bool,
}

impl DelayBased {
    pub fn new(start: u64) -> Self {
        Self {
            bitrate: start as f64,
            trend: Trendline::default(),
            current: None,
            previous: None,
            acked: VecDeque::new(),
            capacity: None,
            updated: None,
            holding: false,
        }
    }

    pub fn bitrate(&self) -> u64 {
        self.bitrate as u64
    }

    /// What the delay last said.
    pub fn signal(&self) -> Signal {
        self.trend.signal
    }

    /// The bit/s the server acknowledged over the last half second, once
    /// there's enough to tell.
    pub fn acked(&self) -> Option<f64> {
        let (first, last) = (self.acked.front()?.0, self.acked.back()?.0);
        let span = last - first;
        (span >= ACKED_WINDOW_US / 2).then(|| {
            let bytes: u64 = self.acked.iter().map(|&(_, size)| u64::from(size)).sum();
            bytes as f64 * 8.0 * 1e6 / span as f64
        })
    }

    /// Takes one feedback's packets, as [`SendHistory::arrivals`] gives
    /// them, at `now` in this side's microseconds, and gives the bit/s to
    /// aim for, between `floor` and `cap`.
    pub fn update(&mut self, arrivals: &[(i64, i64, u32)], now: i64, floor: u64, cap: u64) -> u64 {
        let mut signal = None;
        for &(sent, arrival, size) in arrivals {
            self.acked.push_back((arrival, size));
            match self.current.as_mut() {
                None => {
                    self.current = Some(Group {
                        first_sent: sent,
                        last_sent: sent,
                        last_arrival: arrival,
                    })
                }
                // Reordered: too late for its group.
                Some(group) if sent < group.first_sent => {}
                Some(group) if sent - group.first_sent <= GROUP_SPAN_US => {
                    group.last_sent = group.last_sent.max(sent);
                    group.last_arrival = group.last_arrival.max(arrival);
                }
                Some(group) => {
                    let finished = *group;
                    if let Some(previous) = self.previous {
                        let send_delta = (finished.last_sent - previous.last_sent) as f64 / 1000.0;
                        let arrival_delta =
                            (finished.last_arrival - previous.last_arrival) as f64 / 1000.0;
                        signal = Some(self.trend.update(
                            arrival_delta,
                            send_delta,
                            finished.last_arrival as f64 / 1000.0,
                        ));
                    }
                    self.previous = Some(finished);
                    self.current = Some(Group {
                        first_sent: sent,
                        last_sent: sent,
                        last_arrival: arrival,
                    });
                }
            }
        }
        if let Some(&(newest, _)) = self.acked.back() {
            while self
                .acked
                .front()
                .is_some_and(|&(arrival, _)| newest - arrival > ACKED_WINDOW_US)
            {
                self.acked.pop_front();
            }
        }
        let acked = self.acked();
        let elapsed = self.updated.map_or(0.0, |updated| {
            ((now - updated) as f64 / 1e6).clamp(0.0, 1.0)
        });
        self.updated = Some(now);
        match signal.unwrap_or(self.trend.signal) {
            Signal::Overuse => {
                let base = acked.unwrap_or(self.bitrate);
                // Only ever lower, as when the encoder sent less than aimed.
                self.bitrate = (BETA * base).min(self.bitrate);
                self.capacity = Some(
                    self.capacity
                        .map_or(base, |capacity| 0.95 * capacity + 0.05 * base),
                );
                self.holding = true;
            }
            Signal::Underuse => self.holding = true,
            Signal::Normal if self.holding => self.holding = false,
            Signal::Normal => {
                // The path carries more than the last overuse showed.
                if let (Some(capacity), Some(acked)) = (self.capacity, acked)
                    && acked > capacity * 1.2
                {
                    self.capacity = None;
                }
                let near = self
                    .capacity
                    .is_some_and(|capacity| self.bitrate >= 0.9 * capacity);
                let growth: f64 = if near { 1.02 } else { 1.08 };
                let mut next = self.bitrate * growth.powf(elapsed) + 1000.0 * elapsed;
                // Only as fast as what's acknowledged shows gets through, as
                // libwebrtc's AIMD limits it, overuse or not: a calm screen
                // sends far below its target, and growing past what it
                // sends let the next busy scene burst into loss. Up to the
                // capacity the last overuse showed, it may grow on.
                if let Some(acked) = acked {
                    let limit = 1.5 * acked + 10_000.0;
                    next = next.min(self.capacity.map_or(limit, |capacity| capacity.max(limit)));
                }
                self.bitrate = self.bitrate.max(next);
            }
        }
        self.bitrate = self.bitrate.clamp(floor.min(cap) as f64, cap as f64);
        self.bitrate as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A feedback packet reporting `received` packets from `base`, each
    /// `delta_us` after the one before, with the reference time at 0.
    fn feedback(base: u16, statuses: &[Option<u8>]) -> Vec<u8> {
        let mut packet = vec![0x80 | 15, 205, 0, 0];
        packet.extend_from_slice(&1u32.to_be_bytes());
        packet.extend_from_slice(&2u32.to_be_bytes());
        packet.extend_from_slice(&base.to_be_bytes());
        packet.extend_from_slice(&(statuses.len() as u16).to_be_bytes());
        packet.extend_from_slice(&[0, 0, 0, 7]);
        // Status vector chunks of seven two-bit symbols.
        for chunk in statuses.chunks(7) {
            let mut word: u16 = 0xc000;
            for (index, status) in chunk.iter().enumerate() {
                let symbol = match status {
                    None => 0,
                    Some(_) => 1,
                };
                word |= symbol << ((6 - index) * 2);
            }
            packet.extend_from_slice(&word.to_be_bytes());
        }
        for status in statuses.iter().flatten() {
            packet.push(*status);
        }
        while packet.len() % 4 != 0 {
            packet.push(0);
        }
        let words = (packet.len() / 4 - 1) as u16;
        packet[2..4].copy_from_slice(&words.to_be_bytes());
        packet
    }

    #[test]
    fn reads_which_packets_arrived_and_when() {
        let packet = feedback(65534, &[Some(4), None, Some(8), Some(0)]);
        let reports = transport_feedback(&packet);
        assert_eq!(
            reports,
            [vec![
                (65534, Some(1000)),
                (65535, None),
                (0, Some(3000)),
                (1, Some(3000))
            ]],
            "deltas in 250 µs, wrapping sequence numbers"
        );
        // A run-length chunk: five packets received with small deltas.
        let mut run = vec![
            0x80 | 15,
            205,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            2,
            0,
            10,
            0,
            5,
            0,
            0,
            1,
            0,
        ];
        run.extend_from_slice(&(0x2000u16 | 5).to_be_bytes());
        run.extend_from_slice(&[4, 4, 4, 4, 4, 0]);
        let words = (run.len() / 4 - 1) as u16;
        run[2..4].copy_from_slice(&words.to_be_bytes());
        let report = &transport_feedback(&run)[0];
        assert_eq!(report.len(), 5);
        assert_eq!(
            report[0],
            (10, Some(64_000 + 1000)),
            "the reference time in 64 ms"
        );
        assert_eq!(report[4], (14, Some(64_000 + 5000)));
        assert!(
            transport_feedback(&[0x81, 201, 0, 1, 0, 0, 0, 1]).is_empty(),
            "not feedback"
        );
    }

    #[test]
    fn matches_feedback_to_what_was_sent() {
        let mut history = SendHistory::default();
        history.record(7, 100, 1200);
        history.record(8, 200, 1100);
        let arrivals =
            history.arrivals(&[(8, Some(5200)), (7, Some(5100)), (9, Some(5300)), (6, None)]);
        assert_eq!(arrivals, [(100, 5100, 1200), (200, 5200, 1100)]);
    }

    /// Sends a frame of 10 packets every 10 ms at `rate` for `seconds`, the
    /// path adding `extra_us` more queueing delay with each frame, and
    /// gives feedback every 100 ms. Returns the estimator.
    fn run(start: u64, seconds: i64, extra_us: i64, cap: u64) -> DelayBased {
        let mut estimate = DelayBased::new(start);
        let mut pending = vec![];
        let mut queue = 0;
        for frame in 0..seconds * 100 {
            let sent = frame * 10_000;
            queue += extra_us;
            for packet in 0..10 {
                let at = sent + packet * 100;
                pending.push((at, at + 20_000 + queue + packet * 100, 1200));
            }
            if frame % 10 == 9 {
                estimate.update(&pending, sent, 500_000, cap);
                pending.clear();
            }
        }
        estimate
    }

    #[test]
    fn grows_on_a_clear_path() {
        let estimate = run(4_000_000, 5, 0, 20_000_000);
        assert_eq!(estimate.signal(), Signal::Normal);
        assert!(estimate.bitrate() > 5_000_000, "{}", estimate.bitrate());
        assert!(
            run(4_000_000, 60, 0, 20_000_000).bitrate() <= 20_000_000,
            "up to the cap"
        );
    }

    #[test]
    fn grows_no_further_than_what_gets_through() {
        // A calm screen: 250-byte packets, 2 Mbit/s acknowledged.
        let mut estimate = DelayBased::new(4_000_000);
        for second in 0..30 {
            let mut arrivals = vec![];
            for frame in 0..100 {
                for packet in 0..10 {
                    let at = second * 1_000_000 + frame * 10_000 + packet * 100;
                    arrivals.push((at, at + 20_000, 250));
                }
            }
            for report in arrivals.chunks(100) {
                estimate.update(report, report[0].0, 500_000, 20_000_000);
            }
        }
        assert_eq!(estimate.signal(), Signal::Normal);
        // It grows only in the quarter second before what's acknowledged
        // can be measured.
        assert!(
            estimate.bitrate() < 4_100_000,
            "above 1.5 times 2 Mbit/s already: held, {}",
            estimate.bitrate()
        );
    }

    #[test]
    fn backs_off_as_a_queue_builds() {
        // 1 ms more delay every frame: a queue filling.
        let estimate = run(10_000_000, 3, 1000, 20_000_000);
        assert_eq!(estimate.signal(), Signal::Overuse);
        // 85% of the 9.6 Mbit/s acknowledged.
        assert!(estimate.bitrate() < 9_000_000, "{}", estimate.bitrate());
        assert!(estimate.capacity.is_some());
    }

    #[test]
    fn stays_within_floor_and_cap() {
        let estimate = run(100_000, 1, 0, 20_000_000);
        assert!(estimate.bitrate() >= 500_000);
        let estimate = run(30_000_000, 1, 0, 3_500_000);
        assert_eq!(estimate.bitrate(), 3_500_000);
    }
}
