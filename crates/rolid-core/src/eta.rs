//! Remaining-time estimate for queued operations: once a second a sample of
//! the processed bytes; speed = bytes of the last 10 seconds / 10.

use std::collections::VecDeque;
use std::time::Duration;

use rolid_text::{txt, txtn, Text};

/// Samples (seconds) in the window; the first estimate needs a full window.
pub const WINDOW: usize = 10;

/// Sliding window of per-second byte counts.
#[derive(Clone, Debug, Default)]
pub struct Window {
    /// Bytes processed in each of the last seconds (oldest first).
    deltas: VecDeque<u64>,
    /// Time and byte count of the last sample.
    last: Option<(Duration, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Eta {
    /// Fewer than [`WINDOW`] samples yet.
    Computing,
    /// No progress in the window (paused, connection lost).
    Unknown,
    Secs(u64),
}

impl Window {
    /// Record the processed byte count `done` at time `t` (since the start of
    /// the operation). Takes a sample for every whole second elapsed; when
    /// several seconds passed between calls (a busy UI) their bytes are spread
    /// evenly over them.
    pub fn sample(&mut self, t: Duration, done: u64) {
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
            self.deltas.push_back(part);
            if self.deltas.len() > WINDOW {
                self.deltas.pop_front();
            }
        }
        self.last = Some((lt + Duration::from_secs(secs), done));
    }

    pub fn samples(&self) -> usize {
        self.deltas.len()
    }

    /// Bytes per second over the window (`None` before the first full window).
    pub fn speed(&self) -> Option<f64> {
        (self.deltas.len() >= WINDOW).then(|| self.deltas.iter().sum::<u64>() as f64 / WINDOW as f64)
    }

    /// Remaining time for `remaining` bytes.
    pub fn eta(&self, remaining: u64) -> Eta {
        match self.speed() {
            None => Eta::Computing,
            Some(s) if s <= 0.0 => Eta::Unknown,
            Some(s) => Eta::Secs((remaining as f64 / s).ceil() as u64),
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

/// 45 → "45 s", 750 → "12 min 30 s", 3900 → "1 h 05 min".
pub fn format_secs(s: u64) -> Text {
    if s < 60 {
        txtn!("{n} s", "{n} s", s)
    } else if s < 3600 {
        txt!("{min} min {sec} s", min = s / 60, sec = s % 60)
    } else {
        txt!("{h} h {min} min", h = s / 3600, min = format!("{:02}", (s % 3600) / 60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn needs_ten_samples() {
        let mut w = Window::default();
        for t in 0..10 {
            w.sample(secs(t), t * 1000);
        }
        assert_eq!(w.samples(), 9);
        assert_eq!(w.eta(5000), Eta::Computing);
        assert_eq!(format_eta(w.eta(5000)).to_string(), "remaining: calculating");
        assert_eq!(w.speed(), None);
        w.sample(secs(10), 10_000);
        assert_eq!(w.speed(), Some(1000.0));
        assert_eq!(w.eta(5000), Eta::Secs(5));
    }

    #[test]
    fn steady_speed() {
        let mut w = Window::default();
        for t in 0..=30 {
            // Several calls per second do not add samples.
            w.sample(secs(t), t * 2_000_000);
            w.sample(secs(t) + Duration::from_millis(500), t * 2_000_000 + 1_000_000);
        }
        assert_eq!(w.samples(), WINDOW);
        assert_eq!(w.speed(), Some(2_000_000.0));
        assert_eq!(w.eta(120_000_000), Eta::Secs(60));
        assert_eq!(format_eta(w.eta(120_000_000)).to_string(), "remaining: 1 min 0 s");
    }

    #[test]
    fn fluctuating_speed_uses_the_last_ten_seconds() {
        let mut w = Window::default();
        let mut done = 0;
        w.sample(secs(0), 0);
        // 10 s fast, then 10 s alternating 0 and 2 MB: only the second part counts.
        for t in 1..=10 {
            done += 10_000_000;
            w.sample(secs(t), done);
        }
        for t in 11..=20 {
            done += if t % 2 == 0 { 2_000_000 } else { 0 };
            w.sample(secs(t), done);
        }
        assert_eq!(w.speed(), Some(1_000_000.0));
        assert_eq!(w.eta(90_000_000), Eta::Secs(90));
    }

    #[test]
    fn a_gap_is_spread_over_its_seconds() {
        let mut w = Window::default();
        w.sample(secs(0), 0);
        w.sample(secs(10), 1_000);
        assert_eq!(w.samples(), 10);
        assert_eq!(w.speed(), Some(100.0));
    }

    #[test]
    fn zero_speed_is_unknown() {
        let mut w = Window::default();
        w.sample(secs(0), 0);
        for t in 1..=5 {
            w.sample(secs(t), 1_000 * t);
        }
        // Paused / connection lost for 10 s.
        for t in 6..=15 {
            w.sample(secs(t), 5_000);
        }
        assert_eq!(w.eta(10), Eta::Unknown);
        assert_eq!(format_eta(w.eta(10)).to_string(), "remaining: unknown");
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
        assert_eq!(format_eta(Eta::Secs(45)).to_string(), "remaining: 45 s");
    }
}
