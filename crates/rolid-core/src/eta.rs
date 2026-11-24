//! Remaining-time estimate for queued operations.
//!
//! Once a second a sample of the processed bytes. Two numbers come out of it:
//!
//! - the current speed: the bytes of the last [`WINDOW`] seconds / [`WINDOW`];
//! - the remaining time, which has to be calm: a speed averaged over a longer
//!   horizon (an exponential average whose horizon grows with the transfer up
//!   to [`HORIZON`] seconds), and a shown value that counts down by itself,
//!   slowly corrects a small offset without ever going up, and glides towards
//!   a new estimate only when it is off by more than a tenth.
//!   Nothing is shown in the first [`WARMUP`] seconds, and until [`SETTLED`]
//!   seconds only once the speed of the last five seconds agrees with the
//!   average.
//!
//! A pause of [`WINDOW`] seconds or more without a byte (a lost connection)
//! shows "unknown" and is left out of the average; the shown time stays where
//! it was and goes on when the bytes come again.

use std::collections::VecDeque;
use std::time::Duration;

use rolid_text::{txt, txtn, Text};

/// Seconds in the window of the current speed; also the pause after which the
/// remaining time is unknown.
pub const WINDOW: usize = 10;
/// No remaining time before this many seconds with progress.
pub const WARMUP: u64 = 10;
/// After this many seconds the remaining time is shown even when the speed
/// still changes.
pub const SETTLED: u64 = 30;
/// Longest horizon of the averaged speed, in seconds.
pub const HORIZON: u64 = 60;
/// The shown time is left to count down while the estimate is within this
/// fraction of it (plus [`BAND_SECS`]).
const BAND: f64 = 0.1;
const BAND_SECS: f64 = 2.0;
/// Inside the band it moves this part of the difference a second (at most a
/// second either way)…
const DRIFT: f64 = 0.05;
/// …outside it this part…
const GLIDE: f64 = 0.15;
/// …but never more than this part of the larger of the two (plus a second).
const STEP_MAX: f64 = 0.1;
/// The speed of the last `RECENT` seconds and the averaged one agree when
/// within this fraction.
const AGREE: f64 = 0.25;
const RECENT: usize = 5;

/// Samples of an operation and the estimate shown for it.
#[derive(Clone, Debug, Default)]
pub struct Window {
    /// Bytes processed in each of the last seconds (oldest first).
    deltas: VecDeque<u64>,
    /// Time and byte count of the last sample.
    last: Option<(Duration, u64)>,
    /// Seconds fed into the averaged speed.
    fed: u64,
    /// Averaged speed, bytes a second.
    avg: f64,
    /// Seconds without a byte since the last byte (not fed yet).
    idle: u64,
    /// Remaining time shown, seconds.
    shown: Option<f64>,
    /// Bytes left at the last sample (`None`: total unknown).
    remaining: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Eta {
    /// Not enough samples yet.
    Computing,
    /// No progress for [`WINDOW`] seconds (paused, connection lost).
    Unknown,
    Secs(u64),
}

impl Window {
    /// Record the processed byte count `done` and the bytes still to do
    /// (`None` when the total is not known) at time `t` since the start of
    /// the operation. Takes a sample for every whole second elapsed; when
    /// several seconds passed between calls (a busy UI) their bytes are spread
    /// evenly over them.
    pub fn sample(&mut self, t: Duration, done: u64, remaining: Option<u64>) {
        self.remaining = remaining;
        let Some((lt, ld)) = self.last else {
            self.last = Some((t, done));
            return;
        };
        let secs = t.saturating_sub(lt).as_secs();
        if secs == 0 {
            return;
        }
        let bytes = done.saturating_sub(ld);
        for i in 0..secs {
            // Even split, the remainder in the first samples.
            let part = bytes / secs + u64::from(i < bytes % secs);
            self.second(part);
        }
        self.last = Some((lt + Duration::from_secs(secs), done));
    }

    /// One second with `bytes` processed.
    fn second(&mut self, bytes: u64) {
        self.deltas.push_back(bytes);
        if self.deltas.len() > WINDOW {
            self.deltas.pop_front();
        }
        if bytes == 0 {
            self.idle += 1;
        } else {
            // A short pause belongs to the speed; a long one (an outage) not.
            if self.idle < WINDOW as u64 {
                for _ in 0..self.idle {
                    self.feed(0.0);
                }
            }
            self.idle = 0;
            self.feed(bytes as f64);
        }
        if self.stalled() {
            return; // The shown time waits for the bytes to come again.
        }
        self.update_shown();
    }

    fn feed(&mut self, bytes: f64) {
        self.fed += 1;
        // The mean of all seconds at first, then an exponential average.
        let horizon = self.fed.min(HORIZON) as f64;
        self.avg += (bytes - self.avg) / horizon;
    }

    fn stalled(&self) -> bool {
        self.idle >= WINDOW as u64
    }

    fn update_shown(&mut self) {
        let (Some(rem), true) = (self.remaining, self.avg > 0.0) else {
            self.shown = None;
            return;
        };
        let raw = rem as f64 / self.avg;
        self.shown = match self.shown {
            None => self.ready().then_some(raw),
            Some(s) => {
                let pred = (s - 1.0).max(0.0);
                let diff = raw - pred;
                if diff.abs() <= BAND * pred + BAND_SECS {
                    // Close enough: the clock runs between standing still and
                    // double speed, so a small offset goes away without the
                    // number ever going up.
                    Some((pred + (diff * DRIFT).clamp(-1.0, 1.0)).max(0.0))
                } else {
                    let cap = STEP_MAX * pred.max(raw) + 1.0;
                    Some((pred + (diff * GLIDE).clamp(-cap, cap)).max(0.0))
                }
            }
        };
    }

    /// Enough seconds, and the recent speed agrees with the average (or the
    /// transfer has run long enough not to wait for that).
    fn ready(&self) -> bool {
        if self.fed < WARMUP {
            return false;
        }
        if self.fed >= SETTLED {
            return true;
        }
        let recent = self.deltas.iter().rev().take(RECENT).map(|&d| d as f64).sum::<f64>() / RECENT as f64;
        (recent - self.avg).abs() <= AGREE * self.avg
    }

    pub fn samples(&self) -> usize {
        self.deltas.len()
    }

    /// Bytes per second over the last [`WINDOW`] seconds (`None` before the
    /// first full window).
    pub fn speed(&self) -> Option<f64> {
        // Summed as floats: ten huge jumps (a resumed part counted at once) must not overflow.
        (self.deltas.len() >= WINDOW).then(|| self.deltas.iter().map(|&d| d as f64).sum::<f64>() / WINDOW as f64)
    }

    /// Bytes per second averaged over the longer horizon of the remaining
    /// time (`None` before the first byte).
    pub fn average(&self) -> Option<f64> {
        (self.fed > 0).then_some(self.avg)
    }

    /// The remaining time to show.
    pub fn eta(&self) -> Eta {
        if self.stalled() && self.fed > 0 {
            return Eta::Unknown;
        }
        match self.shown {
            // Rounded: a correction by a fraction of a second must not show as a second more.
            Some(s) => Eta::Secs(s.round().min(u64::MAX as f64) as u64),
            None => Eta::Computing,
        }
    }
}

/// "remaining: 45 s", "remaining: 12 min 30 s", "remaining: 1 h 05 min",
/// "remaining: calculating", "remaining: unknown".
pub fn format_eta(e: Eta) -> Text {
    match e {
        Eta::Computing => txt!("remaining: calculating"),
        Eta::Unknown => txt!("remaining: unknown"),
        Eta::Secs(s) => txt!("remaining: {time}", time = format_secs(s)),
    }
}

/// Beyond this many days the remaining time is not worth counting (a petabyte
/// over a stalled connection would take thousands of years).
pub const DAYS_MAX: u64 = 999;

/// 45 → "45 s", 750 → "12 min 30 s", 3900 → "1 h 05 min", 400 000 →
/// "4 d 15 h", more than [`DAYS_MAX`] days → "over 999 days".
pub fn format_secs(s: u64) -> Text {
    if s < 60 {
        txtn!("{n} s", "{n} s", s)
    } else if s < 3600 {
        txt!("{min} min {sec} s", min = s / 60, sec = s % 60)
    } else if s < 100 * 3600 {
        txt!("{h} h {min} min", h = s / 3600, min = format!("{:02}", (s % 3600) / 60))
    } else if s / 86400 <= DAYS_MAX {
        txt!("{d} d {h} h", d = s / 86400, h = (s % 86400) / 3600)
    } else {
        txtn!("over {n} day", "over {n} days", DAYS_MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// Feeds a transfer of `size` bytes whose speed in second `t` is
    /// `speed(t)`; returns what was shown each second (`None`: no number).
    fn run(size: u64, speed: impl Fn(u64) -> u64) -> Vec<Option<u64>> {
        let mut w = Window::default();
        let mut done = 0u64;
        let mut out = Vec::new();
        w.sample(secs(0), 0, Some(size));
        let mut t = 0;
        while done < size {
            t += 1;
            done = (done + speed(t)).min(size);
            w.sample(secs(t), done, Some(size - done));
            out.push(match w.eta() {
                Eta::Secs(s) => Some(s),
                _ => None,
            });
        }
        out
    }

    /// The largest rise of the shown time from one second to the next.
    fn worst_rise(shown: &[Option<u64>]) -> u64 {
        shown.windows(2).filter_map(|w| Some(w[1]?.saturating_sub(w[0]?))).max().unwrap_or(0)
    }

    /// The largest error against the real remaining time, as a fraction, from
    /// the first number shown until `until` of the run.
    fn worst_error(shown: &[Option<u64>], until: f64) -> f64 {
        let n = shown.len();
        shown
            .iter()
            .enumerate()
            .take((n as f64 * until) as usize)
            .filter_map(|(i, s)| {
                let real = (n - 1 - i) as f64;
                Some((s.as_ref().copied()? as f64 - real).abs() / real.max(1.0))
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn nothing_in_the_first_ten_seconds() {
        let mut w = Window::default();
        for t in 0..10 {
            w.sample(secs(t), t * 1000, Some(100_000 - t * 1000));
            assert_eq!(w.eta(), Eta::Computing, "second {t}");
        }
        assert_eq!(w.samples(), 9);
        assert_eq!(format_eta(w.eta()).to_string(), "remaining: calculating");
        assert_eq!(w.speed(), None);
        w.sample(secs(10), 10_000, Some(90_000));
        assert_eq!(w.speed(), Some(1000.0));
        assert_eq!(w.eta(), Eta::Secs(90));
    }

    #[test]
    fn steady_speed_counts_down_a_second_a_second() {
        let mut w = Window::default();
        for t in 0..=30 {
            // Several calls per second do not add samples.
            w.sample(secs(t), t * 2_000_000, Some(180_000_000 - t * 2_000_000));
            w.sample(secs(t) + Duration::from_millis(500), t * 2_000_000 + 1_000_000, Some(179_000_000 - t * 2_000_000));
        }
        assert_eq!(w.samples(), WINDOW);
        assert_eq!(w.speed(), Some(2_000_000.0));
        assert_eq!(w.eta(), Eta::Secs(60));
        assert_eq!(format_eta(w.eta()).to_string(), "remaining: 1 min 0 s");
        let shown = run(1_000_000_000, |_| 1_000_000);
        assert!(shown.iter().take(9).all(Option::is_none));
        for (i, s) in shown.iter().enumerate().skip(9) {
            assert_eq!(*s, Some(999 - i as u64), "second {}", i + 1);
        }
    }

    #[test]
    fn a_slow_start_waits_for_the_speed_to_settle() {
        // A line that needs 8 seconds to reach its speed: the first number
        // comes only once the recent speed agrees with the average, and it is
        // never far off afterwards.
        let shown = run(20_000_000_000, |t| 10_000_000 * t.min(8));
        let first = shown.iter().position(Option::is_some).unwrap() + 1;
        assert!(first > 10, "first number after {first} s");
        assert!(worst_error(&shown, 0.9) < 0.35, "{}", worst_error(&shown, 0.9));
        assert_eq!(worst_rise(&shown), 0, "{shown:?}");
    }

    #[test]
    fn a_speed_that_jumps_up_and_down_does_not_move_the_number_much() {
        // 30 and 110 MB/s in turns of 10 seconds: the old estimate from the last
        // 10 seconds jumped between about 0.5x and 2x the real time.
        // The first ten seconds are all slow, so the first number is too high;
        // from the second minute on it stays close.
        let shown = run(40_000_000_000, |t| if (t / 10) % 2 == 0 { 30_000_000 } else { 110_000_000 });
        assert!(worst_error(&shown[120..], 0.9) < 0.15, "{}", worst_error(&shown[120..], 0.9));
        assert_eq!(worst_rise(&shown), 0, "{:?}", &shown[..60]);
    }

    #[test]
    fn slow_first_then_fast_glides_without_jumps() {
        // The shape of the 110 GiB upload of 0.8.19: 30 MB/s for 200 s, then
        // 100 MB/s. Nobody can know the speed-up in advance; the number must
        // come down smoothly, never by more than a tenth in a second.
        let shown = run(60_000_000_000, |t| if t <= 200 { 30_000_000 } else { 100_000_000 });
        for w in shown.windows(2) {
            if let (Some(a), Some(b)) = (w[0], w[1]) {
                assert!(b <= a, "{a} -> {b}");
                assert!(a - b <= a / 10 + 2, "{a} -> {b}");
            }
        }
        // Within 15 % of the truth three minutes after the change.
        let n = shown.len();
        let at = 200 + 180;
        let real = (n - 1 - at) as f64;
        let s = shown[at].unwrap() as f64;
        assert!((s - real).abs() / real < 0.15, "{s} vs {real}");
    }

    #[test]
    fn a_gap_is_spread_over_its_seconds() {
        let mut w = Window::default();
        w.sample(secs(0), 0, Some(10_000));
        w.sample(secs(10), 1_000, Some(9_000));
        assert_eq!(w.samples(), 10);
        assert_eq!(w.speed(), Some(100.0));
        assert_eq!(w.average(), Some(100.0));
        assert_eq!(w.eta(), Eta::Secs(90));
    }

    #[test]
    fn a_petabyte_at_a_byte_a_second_is_not_an_overflow() {
        let mut w = Window::default();
        for t in 0..=10 {
            w.sample(secs(t), t, Some(u64::MAX));
        }
        assert_eq!(w.eta(), Eta::Secs(u64::MAX));
        assert_eq!(format_eta(w.eta()).to_string(), "remaining: over 999 days");
        let mut w = Window::default();
        for t in 0..=10 {
            w.sample(secs(t), t * (u64::MAX / 20), Some(u64::MAX / 2));
        }
        assert!(matches!(w.eta(), Eta::Secs(10 | 11)), "{:?}", w.eta());
        // A count that jumps to the top of u64 within two seconds.
        let mut w = Window::default();
        for t in 0..=10 {
            w.sample(secs(t), t.saturating_mul(1 << 63), Some(1));
        }
        assert!(w.speed().is_some());
    }

    #[test]
    fn an_outage_is_unknown_and_left_out() {
        let mut w = Window::default();
        w.sample(secs(0), 0, Some(100_000));
        for t in 1..=20 {
            w.sample(secs(t), 1_000 * t, Some(100_000 - 1_000 * t));
        }
        assert_eq!(w.eta(), Eta::Secs(80));
        // A short pause belongs to the speed: the number goes on counting down
        // (a little slower, as the estimate does not get shorter meanwhile).
        let mut before = 80;
        for t in 21..=25 {
            w.sample(secs(t), 20_000, Some(80_000));
            let Eta::Secs(s) = w.eta() else { panic!("second {t}: {:?}", w.eta()) };
            assert!(s <= before, "second {t}: {before} -> {s}");
            before = s;
        }
        assert!(before <= 77, "{before}");
        // Connection lost for 2 minutes in all.
        for t in 26..=140 {
            w.sample(secs(t), 20_000, Some(80_000));
        }
        assert_eq!(w.eta(), Eta::Unknown);
        assert_eq!(format_eta(w.eta()).to_string(), "remaining: unknown");
        // Back: the speed is the speed before the outage, and the number goes
        // on from where it stopped (about 75 s; the short pause above counts).
        w.sample(secs(141), 21_000, Some(79_000));
        let Eta::Secs(s) = w.eta() else { panic!("{:?}", w.eta()) };
        assert!((70..=80).contains(&s), "{s}");
        assert!(w.average().unwrap() > 900.0, "{:?}", w.average());
    }

    #[test]
    fn nothing_moving_from_the_start_is_still_calculating() {
        let mut w = Window::default();
        for t in 0..=30 {
            w.sample(secs(t), 0, Some(1000));
        }
        assert_eq!(w.eta(), Eta::Computing);
    }

    #[test]
    fn an_unknown_total_shows_no_number() {
        let mut w = Window::default();
        for t in 0..=20 {
            w.sample(secs(t), t * 1000, None);
        }
        assert_eq!(w.eta(), Eta::Computing);
        assert_eq!(w.speed(), Some(1000.0));
    }

    #[test]
    fn formatting() {
        let f = |s| format_secs(s).to_string();
        assert_eq!(f(0), "0 s");
        assert_eq!(f(59), "59 s");
        assert_eq!(f(60), "1 min 0 s");
        assert_eq!(f(750), "12 min 30 s");
        assert_eq!(f(3599), "59 min 59 s");
        assert_eq!(f(3600), "1 h 00 min");
        assert_eq!(f(3900), "1 h 05 min");
        assert_eq!(f(26 * 3600 + 59 * 60 + 59), "26 h 59 min");
        assert_eq!(f(99 * 3600 + 3599), "99 h 59 min");
        assert_eq!(f(100 * 3600), "4 d 4 h");
        assert_eq!(f(999 * 86400 + 86399), "999 d 23 h");
        assert_eq!(f(1000 * 86400), "over 999 days");
        assert_eq!(f(u64::MAX), "over 999 days");
        assert_eq!(format_eta(Eta::Secs(45)).to_string(), "remaining: 45 s");
    }
}
