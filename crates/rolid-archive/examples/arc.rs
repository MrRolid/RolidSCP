//! Manual check tool: `arc pack OUT PW SRC…`, `arc list ARCH PW`, `arc test ARCH PW`, `arc x ARCH DEST PW`
//! (PW "-" = none).
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use rolid_archive::*;
use secrecy::SecretString;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let cancel = AtomicBool::new(false);
    let on = |_: Tick<'_>| {};
    let ctl = Ctl { cancel: &cancel, on: &on };
    let pw = |s: &str| (s != "-").then(|| SecretString::from(s.to_string()));
    let fmt = |p: &str| Format::from_name(p).expect("unknown format");
    let ro = |s: &str| ReadOpts { password: pw(s), ..ReadOpts::new() };
    match a[0].as_str() {
        "pack" => {
            let srcs: Vec<PathBuf> = a[3..].iter().map(PathBuf::from).collect();
            let r = pack(&srcs, a[1].as_ref(), fmt(&a[1]), Level::Normal, pw(&a[2]).as_ref(), &ctl);
            println!("{r:?}");
        }
        "list" => match list(a[1].as_ref(), fmt(&a[1]), &ro(&a[2])) {
            Ok(l) => {
                for i in &l.items {
                    println!("{:?} {:o} {} {:?} {}", i.kind, i.mode.unwrap_or(0), i.size, i.link, i.path);
                }
                println!("rejected {:?} raw {:?} enc {}", l.rejected, l.raw_names, l.encrypted);
            }
            Err(e) => println!("ERR {e}"),
        },
        "test" => println!("{:?}", test(a[1].as_ref(), fmt(&a[1]), &ro(&a[2]), &ctl)),
        "x" => println!(
            "{:?}",
            extract(a[1].as_ref(), fmt(&a[1]), a[2].as_ref(), &ro(&a[3]), Limits::default(), Exist::Overwrite, None, &ctl)
        ),
        "bench" => {
            // arc bench OUTDIR SRC… : every format and level, MB/s of input and ratio.
            let srcs: Vec<PathBuf> = a[2..].iter().map(PathBuf::from).collect();
            let input: u64 = srcs.iter().map(|p| du(p)).sum();
            for f in Format::PACK {
                for &l in f.levels() {
                    if !bench_wanted(f, l) {
                        continue;
                    }
                    let out = PathBuf::from(&a[1]).join(format!("bench.{}", f.ext()));
                    let t = std::time::Instant::now();
                    let r = bench_pack(&srcs, &out, f, l, &ctl);
                    let s = t.elapsed().as_secs_f64();
                    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                    let _ = std::fs::remove_file(&out);
                    match r {
                        Ok(()) => println!(
                            "{:8} {:10} {:7.1} s {:7.1} MB/s ratio {:5.1} %",
                            f.label(),
                            l.label(),
                            s,
                            input as f64 / 1e6 / s,
                            size as f64 * 100.0 / input as f64
                        ),
                        Err(e) => println!("{} {}: ERR {e}", f.label(), l.label()),
                    }
                }
            }
        }
        _ => eprintln!("?"),
    }
}

fn du(p: &std::path::Path) -> u64 {
    let m = std::fs::symlink_metadata(p).unwrap();
    if m.is_dir() {
        std::fs::read_dir(p).unwrap().flatten().map(|e| du(&e.path())).sum()
    } else {
        m.len()
    }
}

/// `ARC_BENCH=zip,7z` limits the formats.
fn bench_wanted(f: Format, _l: Level) -> bool {
    std::env::var("ARC_BENCH").map_or(true, |v| v.split(',').any(|k| k == f.key()))
}

/// `ARC_THREADS=n` sets the threads (default: cores − 1).
fn bench_pack(srcs: &[PathBuf], out: &std::path::Path, f: Format, l: Level, ctl: &Ctl) -> Result<()> {
    let mut o = rolid_archive::PackOpts::new(f, l);
    if let Some(n) = std::env::var("ARC_THREADS").ok().and_then(|v| v.parse().ok()) {
        o.threads = n;
    }
    rolid_archive::pack_with(srcs, out, &o, ctl).map(|_| ())
}
