#!/usr/bin/env python3
"""Licence notices of the Rust libraries in the binary, as paragraphs of a
Debian copyright file (machine-readable format 1.0).

Input: `cargo about generate --format json` (packaging/about/about.toml).
Every distinct licence text gets one paragraph with the crates that use it;
our own workspace crates (path dependencies) are left out, they are covered
by the first paragraph that build-deb.sh writes itself.

Usage: copyright.py about.json >> copyright
"""

import json
import re
import sys

COPYRIGHT_LINE = re.compile(
    r"^\s*(?:copyright\s*(?:\(c\)|©)?\s*[\d©(]|copyright\s+(?:\(c\)|©)|\(c\)\s*\d|©\s*\d)",
    re.IGNORECASE,
)


def field_text(text):
    """A licence text as continuation lines: one leading space, "." for empty lines."""
    out = []
    for line in text.replace("\t", "    ").splitlines():
        line = line.rstrip()
        out.append(" ." if not line.strip() else " " + line)
    while out and out[-1] == " .":
        out.pop()
    return "\n".join(out)


def main():
    data = json.load(open(sys.argv[1], encoding="utf-8"))
    paragraphs = []
    for lic in data["licenses"]:
        crates = []
        for use in lic["used_by"]:
            c = use["crate"]
            if c.get("source") is None:  # our own crate
                continue
            crates.append(c)
        if not crates:
            continue
        crates.sort(key=lambda c: (c["name"], c["version"]))
        seen = set()
        names = []
        for c in crates:
            n = f'{c["name"]}-{c["version"]}'
            if n not in seen:
                seen.add(n)
                names.append(n)
        holders = []
        for line in lic["text"].splitlines():
            if COPYRIGHT_LINE.match(line) and "notice" not in line.lower():
                h = line.strip()
                if h not in holders:
                    holders.append(h)
        if not holders:
            for c in crates:
                for a in c.get("authors") or []:
                    a = re.sub(r"\s*<[^>]*>", "", a).strip()
                    if a and a not in holders:
                        holders.append(a)
        if not holders:
            holders = ["the authors of " + ", ".join(c["name"] for c in crates)]
        p = "Files: " + "\n ".join(names) + "\n"
        p += "Copyright: " + "\n ".join(holders) + "\n"
        p += "Comment: Rust crates built into /usr/bin/rolidscp (licence notice from cargo-about)\n"
        p += "License: " + lic["id"] + "\n" + field_text(lic["text"])
        paragraphs.append(p)
    if not paragraphs:
        sys.exit("copyright.py: no licences in " + sys.argv[1])
    print()
    print("\n\n".join(paragraphs))


if __name__ == "__main__":
    main()
