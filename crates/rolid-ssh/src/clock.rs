//! The server's clock against this computer's: "Overwrite only older"
//! compares the times of files, and a file changed on a server whose clock
//! is off gets a wrong time.
//!
//! The server is asked with `date -u +%s` (the seconds since 1970 in UTC),
//! only where it has a shell, and only for as long as [`PROBE_TIMEOUT`]
//! allows. A server without a shell, or one that answers with something
//! else, gives no reading (and no warning).
//!
//! [`PROBE_TIMEOUT`]: crate::PROBE_TIMEOUT

use std::time::SystemTime;

use crate::SshConnection;

/// The command that asks the server for its time.
pub const COMMAND: &str = "date -u +%s";

/// How far off a clock may be before it is reported (seconds).
pub const LIMIT_SECS: u64 = 60;

/// The server's clock minus this computer's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockOffset {
    /// Positive: the server is ahead.
    pub secs: i64,
    /// How exact the reading is (half the time the answer took, plus the
    /// second the server's answer is rounded to).
    pub uncertainty: u64,
}

impl ClockOffset {
    /// Off by more than [`LIMIT_SECS`], even counting the uncertainty.
    pub fn is_off(&self) -> bool {
        self.secs.unsigned_abs() > LIMIT_SECS + self.uncertainty
    }
}

fn unix_secs(t: SystemTime) -> f64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    }
}

/// The offset from the server's answer and the local times the question was
/// sent and the answer came. `None` for anything but one whole number.
pub fn parse(out: &str, sent: SystemTime, got: SystemTime) -> Option<ClockOffset> {
    let server: i64 = out.trim().parse().ok()?;
    let (a, b) = (unix_secs(sent), unix_secs(got));
    if !(a.is_finite() && b.is_finite()) || b < a {
        return None;
    }
    let mid = (a + b) / 2.0;
    // The server cut its time to whole seconds: its real time is up to 1 s later.
    let diff = server as f64 + 0.5 - mid;
    if !diff.is_finite() || diff.abs() > i64::MAX as f64 / 2.0 {
        return None;
    }
    let uncertainty = ((b - a) / 2.0 + 1.0).ceil() as u64;
    Some(ClockOffset { secs: diff.round() as i64, uncertainty })
}

/// Ask the server for its time. `None` without a shell, after the probe
/// time limit, on a lost connection or an answer that is not a time.
pub async fn measure(conn: &SshConnection) -> Option<ClockOffset> {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut out = Vec::new();
    let mut take = |b: &[u8]| {
        out.extend_from_slice(b);
        out.len() < 256
    };
    let sent = SystemTime::now();
    let r = conn.exec_probe(COMMAND, &cancel, &mut take).await;
    let got = SystemTime::now();
    match r {
        Ok((Some(0) | None, _)) => parse(&String::from_utf8_lossy(&out), sent, got),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: f64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs_f64(secs)
    }

    #[test]
    fn a_clock_an_hour_ahead_is_off() {
        let o = parse("1700003600\n", at(1_700_000_000.2), at(1_700_000_000.4)).unwrap();
        assert_eq!(o.secs, 3600);
        assert_eq!(o.uncertainty, 2);
        assert!(o.is_off());
        let o = parse("1699996400", at(1_700_000_000.0), at(1_700_000_000.1)).unwrap();
        assert_eq!(o.secs, -3600, "behind");
        assert!(o.is_off());
    }

    #[test]
    fn a_minute_and_a_slow_answer_are_not_off() {
        // 59 s ahead: within the limit.
        assert!(!parse("1700000059", at(1_700_000_000.0), at(1_700_000_000.0)).unwrap().is_off());
        // 70 s ahead, but the answer took 30 s: it cannot be told apart.
        let o = parse("1700000085", at(1_700_000_000.0), at(1_700_000_030.0)).unwrap();
        assert!(o.uncertainty >= 16 && !o.is_off(), "{o:?}");
        // The same clocks: about 0.
        let o = parse("1700000000", at(1_700_000_000.1), at(1_700_000_000.2)).unwrap();
        assert!(o.secs.abs() <= 1 && !o.is_off(), "{o:?}");
    }

    #[test]
    fn anything_but_a_number_is_no_reading() {
        let (a, b) = (at(1_700_000_000.0), at(1_700_000_001.0));
        for bad in ["", "Thu Nov 26 10:00:00 UTC 2026", "12 34", "-", "1e9", "99999999999999999999999", "date: not found"] {
            assert_eq!(parse(bad, a, b), None, "{bad:?}");
        }
        // The answer came "before" the question (the local clock was set back).
        assert_eq!(parse("1700000000", b, a), None);
        // Extreme values do not overflow.
        assert!(parse(&i64::MAX.to_string(), a, b).is_none_or(|o| o.is_off()));
        assert!(parse(&i64::MIN.to_string(), a, b).is_none_or(|o| o.is_off()));
    }
}
