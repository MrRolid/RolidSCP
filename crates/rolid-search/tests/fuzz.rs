//! Fuzzing of the records `find -printf` sends back from a server during a
//! remote search (type, target type, size, time, mode, path). A server can
//! send anything: a record must give an entry or nothing, never a panic.
//!
//! See `crates/rolid-core/tests/support/mutate.rs` (ROLIDSCP_FUZZ_ITERS).

#[path = "../../rolid-core/tests/support/mutate.rs"]
mod mutate;

use rolid_search::remote::parse_record;

#[test]
fn fuzz_find_records() {
    let seeds: Vec<Vec<u8>> = [
        "f\tf\t1234\t1700000000.1234567890\t644\t/home/alex/a.txt",
        "d\td\t4096\t1600000000.0000000000\t755\t/home/alex/dir with\ttab",
        "l\td\t7\t1500000000.5\t777\t/home/alex/link",
        "l\tN\t7\t0.0\t777\t/home/alex/broken",
        "p\tp\t0\t1.0\t600\t/tmp/fifo",
    ]
    .iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    let dict: &[&[u8]] = &[b"\t", b"inf", b"-inf", b"1e300", b"NaN", b"-1", b"99999999999999999999", b"0x10", b"7777777777777", b"\xff\xfe"];
    let check = |b: &[u8]| {
        if let Some((path, e)) = parse_record(b) {
            assert!(path.ends_with(&e.name) || e.name.is_empty());
        }
    };
    if mutate::replay(check) {
        return;
    }
    mutate::run("find-record", &seeds, dict, check);
}
