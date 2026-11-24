#!/usr/bin/env python3
"""Table of the load test results with the limits of roadmap 0.8.15.

  report.py [--check] RESULTS.tsv [BEFORE.tsv]

RESULTS.tsv holds `BENCH <tab> name <tab> value <tab> unit` lines (measure.sh).
With BEFORE.tsv a column with the earlier values is added. With --check the
exit status is 1 when a limit is not met (CI).

A limit "≤ 2 × <standard tool>" is checked only when the tool took at least
MIN_REF_MS: below that, starting a process and timer noise decide (small runs
in CI).
"""
import math
import sys


def load(path):
    vals = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.rstrip("\n").split("\t")
            if len(p) >= 3 and p[0] == "BENCH":
                try:
                    vals[p[1]] = float(p[2])
                except ValueError:
                    pass
    return vals


MIN_REF_MS = 200


def rel(v, key, factor):
    """x ≤ factor × the standard tool; None = not compared (no or too short a reference)."""
    r = v.get(key)
    if r is None or r < MIN_REF_MS:
        return lambda x: None
    return lambda x: x <= factor * r


def worst(v, keys):
    xs = [v[k] for k in keys if k in v]
    return max(xs) if xs else None


# (label, how to get the value, limit as text, check(value, values) -> bool)
def checks(v):
    ref = lambda k: v.get(k)
    sorts = [f"local.sort.{k}_ms" for k in ("size", "date", "ext", "name", "name_desc")]
    return [
        ("Local folder: listed and shown", v.get("local.list.total_ms"), "≤ 10 000 ms", lambda x: x <= 10000),
        ("Local folder: memory for the listing", v.get("local.list.rss_extra_mb"), "≤ 1 024 MB", lambda x: x <= 1024),
        ("Window: folder shown", v.get("gui.list_shown_ms"), "≤ 10 000 ms", lambda x: x <= 10000),
        ("Window: PgDn, average frame", v.get("gui.pgdn.avg_ms"), "≤ 8 ms", lambda x: x <= 8),
        ("Window: PgDn, longest frame", v.get("gui.pgdn.max_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Window: mouse wheel, average frame", v.get("gui.wheel.avg_ms"), "≤ 8 ms", lambda x: x <= 8),
        ("Window: mouse wheel, longest frame", v.get("gui.wheel.max_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Window: Home/End, longest frame", v.get("gui.ends.max_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Window: all marked, longest frame", v.get("gui.marked.max_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Window: Shift+End from the top, longest frame", v.get("gui.shiftend.max_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Window: Shift+Home back, longest frame", v.get("gui.shifthome.max_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Window: memory (RSS)", v.get("gui.rss_mb"), "≤ 1 024 MB above idle", lambda x: x <= 1024 + 200),
        ("SFTP folder: listed and shown", v.get("sftp.list.total_ms"), "≤ 60 000 ms", lambda x: x <= 60000),
        ("SFTP folder: leaving stops the listing", v.get("sftp.list.abort_ms"), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Sorting (slowest column)", worst(v, sorts), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Quick filter, one key (slowest)", v.get("local.filter.worst_ms"), "≤ 200 ms", lambda x: x <= 200),
        ("Window: quick filter, longest frame", v.get("gui.filter.max_ms"), "≤ 200 ms", lambda x: x <= 200),
        # The footer is kept between frames: these are the frames that make it again.
        ("Footer, first frame of the folder", v.get("local.footer_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Footer after Ctrl+A", v.get("local.footer_marked_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Footer after one more mark (Insert)", v.get("local.footer_one_more_ms"), "≤ 8 ms", lambda x: x <= 8),
        ("Mark all (Ctrl+A)", v.get("local.mark_all_ms"), "≤ 200 ms", lambda x: x <= 200),
        ("Shift+End from the top, with the footer", v.get("local.shift_end_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Shift+End: items marked", v.get("local.shift_end.marked"), "= all", lambda x: x == v.get("local.shift_end.expected")),
        ("Shift+Home back, with the footer", v.get("local.shift_home_back_ms"), "≤ 50 ms", lambda x: x <= 50),
        ("Shift+Home back: items left marked", v.get("local.shift_home_back.marked"), "≤ 1 (the anchor)", lambda x: x <= 1),
        ("Sorting with all marked", v.get("local.sort_all_marked_ms"), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Refresh with all marked, a size changed", v.get("local.refresh_size_changed_all_marked_ms"), "≤ 100 ms", lambda x: x <= 100),
        ("Refresh with all marked, one file gone", v.get("local.refresh_one_gone_all_marked_ms"), "≤ 100 ms", lambda x: x <= 100),
        ("Refresh with all marked: marks kept", v.get("local.refresh_one_gone_all_marked.marked"), "all − 1", lambda x: x == v.get("local.refresh_one_gone_all_marked.expected")),
        ("Background refresh, nothing changed", v.get("local.refresh_same_ms"), "≤ 100 ms", lambda x: x <= 100),
        ("Background refresh, one file more", v.get("local.refresh_changed_ms"), "≤ 100 ms", lambda x: x <= 100),
        ("Folder size, local", v.get("size.local_ms"), "≤ 2 × du -sb", rel(v, "ref.du_sb_ms", 2)),
        ("Folder size, local = the sizes of the files", v.get("size.local.bytes"), "= find -printf %s",
         lambda x: ref("ref.files.bytes") is None or x == ref("ref.files.bytes")),
        ("Folder size, server (du)", v.get("size.server_du_ms"), "≤ 2 × du over ssh", rel(v, "ref.ssh_du_ms", 2)),
        ("Folder size, server = du -sb", v.get("size.server_du.bytes"), "= du -sb", lambda x: ref("ref.du_sb.bytes") is None or x == ref("ref.du_sb.bytes")),
        ("Search by name, local", v.get("search.local.total_ms"), "≤ 2 × find", rel(v, "ref.find_ms", 2)),
        ("Search by name, local, first hit", v.get("search.local.first_ms"), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Search by name, server", v.get("search.server_find.total_ms"), "≤ 2 × find over ssh", rel(v, "ref.ssh_find_ms", 2)),
        ("Search by name, server, hits = local", v.get("search.server_find.hits"), "= local", lambda x: x == v.get("search.local.hits")),
        ("Copy to the server", v.get("copy.up_ms"), "≤ 2 × sftp put -r", rel(v, "ref.sftp_put_ms", 2)),
        ("Copy from the server", v.get("copy.down_ms"), "≤ 2 × sftp get -r", rel(v, "ref.sftp_get_ms", 2)),
        ("Copy there and back: wrong files", sum(v.get(f"copy.roundtrip.{k}", math.nan) for k in ("missing", "different", "extra")), "0", lambda x: x == 0),
        ("Copy: cancel takes effect", v.get("copy.cancel_stop_ms"), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Copy canceled, finished with Skip: wrong files", sum(v.get(f"copy.resume.{k}", math.nan) for k in ("missing", "different", "extra")), "0", lambda x: x == 0),
        ("Delete, local", v.get("delete.local_ms"), "≤ 2 × rm -rf", rel(v, "ref.rm_rf_ms", 2)),
        ("Delete, server", v.get("delete.sftp_ms"), "≤ 2 × sftp rm", rel(v, "ref.sftp_rm_ms", 2)),
        # A fixed few MB of the allocator (one arena per worker thread) are a lot
        # of per cent with a small folder: a leak grows with the folder.
        ("Memory: 10 more listings after the first 2", v.get("leak.rss_growth_pct"), "≤ +10 % or ≤ +32 MB",
         lambda x: x <= 10 or v.get("leak.rss_after_12_mb", 1e9) - v.get("leak.rss_after_2_mb", 0) <= 32),
        ("Open files: 10 more listings", v.get("leak.fd_growth"), "+0", lambda x: x <= 0),
        ("Open files after the copies", v.get("copy.fds_extra"), "≤ +2 (SSH sockets)", lambda x: x <= 2),
        ("Deep tree: files copied", v.get("deep.copy.files"), "2 (both bottoms)", lambda x: x == 2),
    ]


def fmt(x):
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return "–"
    if abs(x) >= 100:
        return f"{x:,.0f}".replace(",", " ")
    return f"{x:.1f}"


def main():
    args = sys.argv[1:]
    check = "--check" in args
    args = [a for a in args if a != "--check"]
    v = load(args[0])
    before = load(args[1]) if len(args) > 1 else None
    failed = []
    head = "| Test | " + ("Before | " if before else "") + "Value | Limit | Result |"
    print(head)
    print("|" + "---|" * (head.count("|") - 1))
    names_b = {}
    if before:
        for label, val, _, _ in checks(before):
            names_b[label] = val
    for label, val, limit, ok in checks(v):
        if val is None or (isinstance(val, float) and math.isnan(val)):
            res = "not measured"
        else:
            r = ok(val)
            if r is None:
                res = f"not compared (reference < {MIN_REF_MS} ms)"
            elif r:
                res = "pass"
            else:
                res = "**FAIL**"
                failed.append(label)
        cols = [label] + ([fmt(names_b.get(label))] if before else []) + [fmt(val), limit, res]
        print("| " + " | ".join(cols) + " |")
    refs = {k: x for k, x in v.items() if k.startswith("ref.")}
    if refs:
        print()
        print("Standard tools on the same machine: " + ", ".join(f"{k[4:]} {fmt(x)}" for k, x in sorted(refs.items())))
    if check and failed:
        print("\nLimits not met: " + "; ".join(failed), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
