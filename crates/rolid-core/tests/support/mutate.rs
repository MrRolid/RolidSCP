//! A small deterministic mutation fuzzer for the tests of what the program
//! reads (archives, the output of commands on a server, `known_hosts`,
//! images and texts in the viewer). It runs on stable Rust inside the
//! normal tests; included by path from each crate's fuzz test.
//!
//! `ROLIDSCP_FUZZ_ITERS` sets the number of mutated inputs per target
//! (default [`DEFAULT_ITERS`], a few seconds in CI; a long run by hand uses
//! 100 000 or more), `ROLIDSCP_FUZZ_SEED` the first seed. An input that
//! panics is saved to `target/fuzz-fail/<target>-<seed>.bin` and the test
//! fails with its seed; an input that takes longer than [`SLOW`] fails too.
#![allow(dead_code)]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const DEFAULT_ITERS: u64 = 300;

/// One input may take this long (a hang or a quadratic blow-up fails).
pub const SLOW: Duration = Duration::from_secs(10);

/// xorshift64*: fast, deterministic, good enough to pick mutations.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Values that often hit edge cases of lengths and counts.
const INTERESTING: &[&[u8]] = &[
    &[0],
    &[0xff],
    &[0x7f],
    &[0x80],
    &[0xff, 0xff],
    &[0xff, 0xff, 0xff, 0xff],
    &[0x00, 0x00, 0x00, 0x80],
    &[0xff, 0xff, 0xff, 0x7f],
    &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
    b"-1",
    b"0",
    b"inf",
    b"NaN",
    b"1e309",
    b"18446744073709551616",
    b"99999999999999999999",
    b"\t",
    b"\n",
    b"/",
    b"..",
    b"\\",
];

/// One mutated copy of `seed` (one to eight changes).
pub fn mutate(rng: &mut Rng, seed: &[u8], dict: &[&[u8]]) -> Vec<u8> {
    let mut d = seed.to_vec();
    for _ in 0..1 + rng.below(8) {
        let len = d.len();
        match rng.below(9) {
            // Flip a bit.
            0 if len > 0 => {
                let i = rng.below(len);
                d[i] ^= 1 << rng.below(8);
            }
            // A random byte.
            1 if len > 0 => {
                let i = rng.below(len);
                d[i] = rng.next() as u8;
            }
            // Insert random bytes.
            2 => {
                let i = rng.below(len + 1);
                let n = 1 + rng.below(16);
                let bytes: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
                d.splice(i..i, bytes);
            }
            // Delete a range.
            3 if len > 1 => {
                let i = rng.below(len);
                let n = 1 + rng.below((len - i).min(64));
                d.drain(i..i + n);
            }
            // Duplicate a range somewhere else.
            4 if len > 1 => {
                let i = rng.below(len);
                let n = 1 + rng.below((len - i).min(64));
                let chunk = d[i..i + n].to_vec();
                let at = rng.below(d.len() + 1);
                d.splice(at..at, chunk);
            }
            // Cut the end off.
            5 if len > 1 => d.truncate(rng.below(len)),
            // Overwrite with an interesting value.
            6 => {
                let v = INTERESTING[rng.below(INTERESTING.len())];
                let i = rng.below(len + 1);
                let end = (i + v.len()).min(len);
                d.splice(i..end, v.iter().copied());
            }
            // Insert a token of the format.
            7 if !dict.is_empty() => {
                let v = dict[rng.below(dict.len())];
                let i = rng.below(len + 1);
                d.splice(i..i, v.iter().copied());
            }
            // Splice in a piece of the seed (repeats structures).
            _ if !seed.is_empty() => {
                let i = rng.below(seed.len());
                let n = 1 + rng.below((seed.len() - i).min(256));
                let at = rng.below(len + 1);
                d.splice(at..at, seed[i..i + n].iter().copied());
            }
            _ => d.push(rng.next() as u8),
        }
    }
    d
}

pub fn iters() -> u64 {
    std::env::var("ROLIDSCP_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_ITERS)
}

fn first_seed() -> u64 {
    std::env::var("ROLIDSCP_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1)
}

fn fail_dir() -> PathBuf {
    // CARGO_TARGET_DIR or <workspace>/target.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    target.join("fuzz-fail")
}

/// Feed `iters()` mutations of the seeds to `check` (also the seeds
/// themselves first). `check` gets the bytes; it must not panic and must
/// finish within [`SLOW`].
pub fn run(name: &str, seeds: &[Vec<u8>], dict: &[&[u8]], mut check: impl FnMut(&[u8])) {
    assert!(!seeds.is_empty(), "{name}: no seeds");
    let start = Instant::now();
    let mut failure = None;
    let first = first_seed();
    let n = iters();
    for k in 0..seeds.len() as u64 + n {
        let (input, seed_no) = if (k as usize) < seeds.len() {
            (seeds[k as usize].clone(), None)
        } else {
            let s = first + k;
            let mut rng = Rng::new(s);
            let base = &seeds[rng.below(seeds.len())];
            (mutate(&mut rng, base, dict), Some(s))
        };
        let t = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| check(&input)));
        let slow = t.elapsed() > SLOW;
        if r.is_err() || slow {
            let dir = fail_dir();
            let _ = std::fs::create_dir_all(&dir);
            let file = dir.join(format!("{name}-{}.bin", seed_no.map_or("seed".to_string(), |s| s.to_string())));
            let _ = std::fs::write(&file, &input);
            let what = if slow { format!("took {:?}", t.elapsed()) } else { "panicked".to_string() };
            failure = Some(format!("{name}: input {} {what}; saved to {}", seed_no.map_or("(a seed)".to_string(), |s| s.to_string()), file.display()));
            break;
        }
    }
    if let Some(f) = failure {
        panic!("{f}");
    }
    eprintln!("FUZZ\t{name}\t{n} inputs\t{:.1} s", start.elapsed().as_secs_f64());
}

/// Feed the input of one saved failure again (by `ROLIDSCP_FUZZ_REPLAY=<file>`).
pub fn replay(mut check: impl FnMut(&[u8])) -> bool {
    match std::env::var_os("ROLIDSCP_FUZZ_REPLAY") {
        Some(f) => {
            check(&std::fs::read(f).expect("replay file"));
            true
        }
        None => false,
    }
}
