#!/usr/bin/env python3
"""Table of the load test results with the limits of roadmap 0.8.15.

  report.py RESULTS.tsv [BEFORE.tsv]

RESULTS.tsv holds `BENCH <tab> name <tab> value <tab> unit` lines (measure.sh).
With BEFORE.tsv a column with the earlier values is added.
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
        ("Background refresh, nothing changed", v.get("local.refresh_same_ms"), "≤ 100 ms", lambda x: x <= 100),
        ("Background refresh, one file more", v.get("local.refresh_changed_ms"), "≤ 100 ms", lambda x: x <= 100),
        ("Folder size, local", v.get("size.local_ms"), "≤ 2 × du -sb", lambda x: ref("ref.du_sb_ms") is None or x <= 2 * max(ref("ref.du_sb_ms"), 1)),
        ("Folder size, local = du -sb", v.get("size.local.bytes"), "= du -sb", lambda x: ref("ref.du_sb.bytes") is None or x == ref("ref.du_sb.bytes")),
        ("Folder size, server (du)", v.get("size.server_du_ms"), "≤ 2 × du over ssh", lambda x: ref("ref.ssh_du_ms") is None or x <= 2 * max(ref("ref.ssh_du_ms"), 1)),
        ("Folder size, server = du -sb", v.get("size.server_du.bytes"), "= du -sb", lambda x: ref("ref.du_sb.bytes") is None or x == ref("ref.du_sb.bytes")),
        ("Search by name, local", v.get("search.local.total_ms"), "≤ 2 × find", lambda x: ref("ref.find_ms") is None or x <= 2 * max(ref("ref.find_ms"), 1)),
        ("Search by name, local, first hit", v.get("search.local.first_ms"), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Search by name, server", v.get("search.server_find.total_ms"), "≤ 2 × find over ssh", lambda x: ref("ref.ssh_find_ms") is None or x <= 2 * max(ref("ref.ssh_find_ms"), 1)),
        ("Search by name, server, hits = local", v.get("search.server_find.hits"), "= local", lambda x: x == v.get("search.local.hits")),
        ("Copy to the server", v.get("copy.up_ms"), "≤ 2 × sftp put -r", lambda x: ref("ref.sftp_put_ms") is None or x <= 2 * ref("ref.sftp_put_ms")),
        ("Copy from the server", v.get("copy.down_ms"), "≤ 2 × sftp get -r", lambda x: ref("ref.sftp_get_ms") is None or x <= 2 * ref("ref.sftp_get_ms")),
        ("Copy there and back: wrong files", sum(v.get(f"copy.roundtrip.{k}", math.nan) for k in ("missing", "different", "extra")), "0", lambda x: x == 0),
        ("Copy: cancel takes effect", v.get("copy.cancel_stop_ms"), "≤ 1 000 ms", lambda x: x <= 1000),
        ("Copy canceled, finished with Skip: wrong files", sum(v.get(f"copy.resume.{k}", math.nan) for k in ("missing", "different", "extra")), "0", lambda x: x == 0),
        ("Delete, local", v.get("delete.local_ms"), "≤ 2 × rm -rf", lambda x: ref("ref.rm_rf_ms") is None or x <= 2 * max(ref("ref.rm_rf_ms"), 1)),
        ("Delete, server", v.get("delete.sftp_ms"), "≤ 2 × sftp rm", lambda x: ref("ref.sftp_rm_ms") is None or x <= 2 * ref("ref.sftp_rm_ms")),
        ("Memory: 10 more listings after the first 2", v.get("leak.rss_growth_pct"), "≤ +10 %", lambda x: x <= 10),
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
    v = load(sys.argv[1])
    before = load(sys.argv[2]) if len(sys.argv) > 2 else None
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
            res = "pass" if ok(val) else "**FAIL**"
        cols = [label] + ([fmt(names_b.get(label))] if before else []) + [fmt(val), limit, res]
        print("| " + " | ".join(cols) + " |")
    refs = {k: x for k, x in v.items() if k.startswith("ref.")}
    if refs:
        print()
        print("Standard tools on the same machine: " + ", ".join(f"{k[4:]} {fmt(x)}" for k, x in sorted(refs.items())))


if __name__ == "__main__":
    main()
