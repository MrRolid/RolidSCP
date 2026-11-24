//! The remaining-time estimate replayed on recorded transfers: the byte count
//! of a real transfer once a second (or so), fed to the estimate of the
//! transfer queue ([`rolid_core::eta::Window`]) and, for comparison, to the
//! estimate of 0.8.19 and older (the bytes of the last 10 seconds only).
//!
//! Ignored by a plain `cargo test`. `ROLIDSCP_ETA_TRACE` is a list of
//! tab-separated files separated by `:`; each has a header line and the
//! columns named by `ROLIDSCP_ETA_COLS` (default `seconds,bytes`: seconds
//! since the start and the bytes transferred). The transfer is taken to end
//! with the last line, its size is the last byte count. One line per file and
//! estimate goes to stdout:
//!
//! `ETA <file> <estimate> <metric> <value>`
//!
//! Metrics, with "real" the time that was really left:
//! - `first_s`: the second the first number was shown
//! - `err_at_<p>pct`: the error against the real time when p % was done
//! - `err_mean`, `err_p90`: mean and 90th percentile of the error between 10
//!   and 90 % done
//! - `rises`: how often the number went up from one second to the next
//! - `rise_max_pct`: the largest rise, in % of the number before
//! - `jumps`: how often the number moved by more than a tenth (plus 2 s) in
//!   one second, either way, beyond counting down

use std::collections::VecDeque;
use std::time::Duration;

use rolid_core::eta::{Eta, Window};

/// The estimate of 0.8.19: the bytes of the last 10 seconds / 10.
#[derive(Default)]
struct Old {
    deltas: VecDeque<u64>,
    last: Option<(Duration, u64)>,
}

impl Old {
    fn sample(&mut self, t: Duration, done: u64) {
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
            self.deltas.push_back(bytes / secs + u64::from(i < bytes % secs));
            if self.deltas.len() > 10 {
                self.deltas.pop_front();
            }
        }
        self.last = Some((lt + Duration::from_secs(secs), done));
    }

    fn eta(&self, remaining: u64) -> Option<u64> {
        if self.deltas.len() < 10 {
            return None;
        }
        let s = self.deltas.iter().map(|&d| d as f64).sum::<f64>() / 10.0;
        (s > 0.0).then(|| (remaining as f64 / s).ceil() as u64)
    }
}

/// (seconds, bytes) of a trace.
fn read(path: &str, cols: &[&str]) -> Vec<(f64, u64)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().expect("header").split('\t').collect();
    let col = |name: &str| head.iter().position(|h| *h == name).unwrap_or_else(|| panic!("{path}: no column {name}"));
    let (ts, bs) = (col(cols[0]), col(cols[1]));
    let mut top = 0;
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[ts].parse().expect("seconds"), f[bs].parse::<f64>().expect("bytes") as u64)
        })
        // A count that went down is a fault of the recording (a part file
        // read while the server was away), not of the transfer: left out.
        .filter(|&(_, b)| {
            let keep = b >= top;
            top = top.max(b);
            keep
        })
        .collect()
}

/// What an estimate showed at each sample: (seconds, bytes, shown).
type Shown = Vec<(f64, u64, Option<u64>)>;

fn metrics(name: &str, which: &str, shown: &Shown) {
    let out = |m: &str, v: f64| println!("ETA\t{name}\t{which}\t{m}\t{v:.3}");
    let (t0, b0) = (shown[0].0, shown[0].1);
    let (end, size) = (shown.last().unwrap().0, shown.last().unwrap().1);
    let err = |&(t, _, s): &(f64, u64, Option<u64>)| -> Option<f64> {
        let real = (end - t).max(1.0);
        Some((s? as f64 - real).abs() / real * 100.0)
    };
    if let Some(f) = shown.iter().find(|s| s.2.is_some()) {
        out("first_s", f.0 - t0);
    }
    for pct in [10u64, 25, 50, 75] {
        let at = b0 + (size - b0) * pct / 100;
        if let Some(e) = shown.iter().find(|s| s.1 >= at && s.2.is_some()).and_then(err) {
            out(&format!("err_at_{pct}pct"), e);
        }
    }
    let lo = b0 + (size - b0) / 10;
    let hi = b0 + (size - b0) * 9 / 10;
    let mut errs: Vec<f64> = shown.iter().filter(|s| s.1 >= lo && s.1 <= hi).filter_map(err).collect();
    if !errs.is_empty() {
        errs.sort_by(f64::total_cmp);
        out("err_mean", errs.iter().sum::<f64>() / errs.len() as f64);
        out("err_p90", errs[errs.len() * 9 / 10]);
    }
    let (mut rises, mut rise_max, mut jumps) = (0u64, 0.0f64, 0u64);
    for w in shown.windows(2) {
        let (Some(a), Some(b)) = (w[0].2, w[1].2) else { continue };
        let dt = w[1].0 - w[0].0;
        let (a, b) = (a as f64, b as f64);
        if b > a {
            rises += 1;
            rise_max = rise_max.max((b - a) / a.max(1.0) * 100.0);
        }
        if (b - (a - dt)).abs() > a * 0.1 + 2.0 {
            jumps += 1;
        }
    }
    out("rises", rises as f64);
    out("rise_max_pct", rise_max);
    out("jumps", jumps as f64);
    out("samples", shown.len() as f64);
}

#[test]
#[ignore]
fn eta_replay() {
    let Ok(files) = std::env::var("ROLIDSCP_ETA_TRACE") else {
        eprintln!("SKIP: ROLIDSCP_ETA_TRACE not set");
        return;
    };
    let cols = std::env::var("ROLIDSCP_ETA_COLS").unwrap_or_else(|_| "seconds,bytes".into());
    let cols: Vec<&str> = cols.split(',').collect();
    for path in files.split(':') {
        let trace = read(path, &cols);
        let size = trace.last().expect("empty trace").1;
        let t0 = trace[0].0;
        let (mut new, mut old) = (Window::default(), Old::default());
        let (mut shown_new, mut shown_old) = (Shown::new(), Shown::new());
        let mut prev: Option<(f64, u64)> = None;
        for &(t, b) in &trace {
            // A gap in the recording (the computer that recorded could not
            // reach the server): the program samples twice a second, so it
            // saw the count standing still until the next line.
            if let Some((pt, pb)) = prev {
                let mut h = pt + 1.0;
                while h < t - 1.0 {
                    let at = Duration::from_secs_f64(h - t0);
                    new.sample(at, pb, Some(size - pb));
                    old.sample(at, pb);
                    h += 1.0;
                }
            }
            prev = Some((t, b));
            let at = Duration::from_secs_f64(t - t0);
            new.sample(at, b, Some(size - b));
            old.sample(at, b);
            shown_new.push((t, b, if let Eta::Secs(s) = new.eta() { Some(s) } else { None }));
            shown_old.push((t, b, old.eta(size - b)));
        }
        let name = path.rsplit('/').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("/");
        metrics(&name, "new", &shown_new);
        metrics(&name, "old", &shown_old);
        if let Ok(dir) = std::env::var("ROLIDSCP_ETA_OUT") {
            // Both estimates second by second, for a chart.
            let mut s = String::from("seconds\tbytes\treal\tnew\told\n");
            let end = trace.last().unwrap().0;
            let opt = |v: Option<u64>| v.map_or(String::new(), |v| v.to_string());
            for (n, o) in shown_new.iter().zip(&shown_old) {
                s += &format!("{:.0}\t{}\t{:.0}\t{}\t{}\n", n.0 - t0, n.1, end - n.0, opt(n.2), opt(o.2));
            }
            std::fs::write(format!("{dir}/{}.tsv", name.replace('/', "-")), s).expect("write");
        }
    }
}
