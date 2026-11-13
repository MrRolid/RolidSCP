//! Shared bandwidth limit (token bucket) for transfers.
//!
//! One [`RateLimit`] is shared by all running transfers, so the limit caps their
//! total throughput (e.g. to keep a slow uplink usable). The rate can be changed
//! while transfers run; `0` means unlimited.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

/// Bytes allowed to accumulate while idle, as a fraction of one second of rate.
const BURST_SECS: f64 = 0.25;

#[derive(Debug)]
pub struct RateLimit {
    /// Bytes per second; 0 = unlimited.
    rate: AtomicU64,
    bucket: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new(0)
    }
}

impl RateLimit {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            rate: AtomicU64::new(bytes_per_sec),
            bucket: Mutex::new(Bucket { tokens: 0.0, last: Instant::now() }),
        }
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Change the limit (takes effect for the next chunk of every transfer).
    pub fn set_rate(&self, bytes_per_sec: u64) {
        self.rate.store(bytes_per_sec, Ordering::Relaxed);
        let mut b = self.bucket.lock().unwrap();
        b.tokens = 0.0;
        b.last = Instant::now();
    }

    /// Chunk size that keeps transfers smooth under the current limit (~100 ms of data).
    pub fn chunk(&self, max: usize) -> usize {
        match self.rate() {
            0 => max,
            r => ((r / 10) as usize).clamp(4096, max),
        }
    }

    /// How long to wait after transferring `n` bytes (debt-based token bucket).
    pub fn reserve(&self, n: u64) -> Duration {
        let rate = self.rate();
        if rate == 0 {
            return Duration::ZERO;
        }
        let rate = rate as f64;
        let mut b = self.bucket.lock().unwrap();
        let now = Instant::now();
        let refill = now.duration_since(b.last).as_secs_f64() * rate;
        b.tokens = (b.tokens + refill).min(rate * BURST_SECS);
        b.last = now;
        b.tokens -= n as f64;
        if b.tokens < 0.0 {
            Duration::from_secs_f64(-b.tokens / rate)
        } else {
            Duration::ZERO
        }
    }

    /// Account for `n` transferred bytes and sleep as needed.
    pub async fn consume(&self, n: u64) {
        let d = self.reserve(n);
        if !d.is_zero() {
            tokio::time::sleep(d).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let r = RateLimit::new(0);
        assert_eq!(r.reserve(1 << 30), Duration::ZERO);
        assert_eq!(r.chunk(256 * 1024), 256 * 1024);
    }

    #[test]
    fn debt_is_shared_between_callers() {
        let r = RateLimit::new(1000);
        // Two "transfers" each send 500 B at once: together one second of budget.
        let a = r.reserve(500);
        let b = r.reserve(500);
        assert!(a.as_secs_f64() >= 0.45 && a.as_secs_f64() <= 0.55, "{a:?}");
        assert!(b.as_secs_f64() >= 0.95 && b.as_secs_f64() <= 1.05, "{b:?}");
        r.set_rate(0);
        assert_eq!(r.reserve(10_000), Duration::ZERO);
        assert_eq!(RateLimit::new(1_000_000).chunk(256 * 1024), 100_000);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn average_rate_is_respected() {
        let r = RateLimit::new(100_000);
        let start = tokio::time::Instant::now();
        for _ in 0..50 {
            let n = r.chunk(256 * 1024) as u64; // 10 kB
            r.consume(n).await;
        }
        // 500 kB at 100 kB/s ≈ 5 s (minus the initial burst allowance).
        let secs = start.elapsed().as_secs_f64();
        assert!((4.5..=5.1).contains(&secs), "{secs}");
    }
}
