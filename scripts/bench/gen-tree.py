#!/usr/bin/env python3
"""Generate file trees for the load tests (scripts/bench/measure.sh).

  gen-tree.py flat DIR COUNT [--size BYTES]
      COUNT files in one folder (f0000000, f0000001, ...).
  gen-tree.py tree DIR COUNT [--per-dir N] [--size BYTES]
      COUNT files spread over folders of N files (d0000/f0000000, ...).
  gen-tree.py deep DIR [--depth N]
      A chain of N nested folders, and a second chain whose path is close to
      PATH_MAX (4096 bytes), each with a file at the bottom; a symlink loop.
  gen-tree.py odd DIR
      Unusual names (invalid UTF-8, line breaks, NFC/NFD twins, leading dash,
      255 bytes...) and special files (FIFO, socket, link to a device).

File content is deterministic (the name repeated), so copies can be compared
with sha256 manifests. Everything is created inside DIR only.
"""

import argparse
import os
import socket
import sys


def content(name: bytes, size: int) -> bytes:
    if size <= 0:
        return b""
    unit = name + b"\n"
    return (unit * (size // len(unit) + 1))[:size]


def write_file(path: bytes, name: bytes, size: int) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    try:
        data = content(name, size)
        if data:
            os.write(fd, data)
    finally:
        os.close(fd)


def flat(d: bytes, count: int, size: int) -> None:
    os.makedirs(d, exist_ok=True)
    for i in range(count):
        name = b"f%07d" % i
        write_file(os.path.join(d, name), name, size)


def tree(d: bytes, count: int, per_dir: int, size: int) -> None:
    os.makedirs(d, exist_ok=True)
    for i in range(count):
        sub = os.path.join(d, b"d%04d" % (i // per_dir))
        if i % per_dir == 0:
            os.mkdir(sub)
        name = b"f%07d" % i
        write_file(os.path.join(sub, name), name, size)


def deep(d: bytes, depth: int) -> None:
    os.makedirs(d, exist_ok=True)
    # 1) Many levels with short names (relative paths: the absolute one may exceed PATH_MAX).
    fd = os.open(d, os.O_RDONLY | os.O_DIRECTORY)
    top = fd
    chain = os.path.join(d, b"levels")
    os.mkdir(chain)
    cur = os.open(chain, os.O_RDONLY | os.O_DIRECTORY)
    for _ in range(depth):
        os.mkdir(b"d", dir_fd=cur)
        nxt = os.open(b"d", os.O_RDONLY | os.O_DIRECTORY, dir_fd=cur)
        os.close(cur)
        cur = nxt
    f = os.open(b"bottom.txt", os.O_WRONLY | os.O_CREAT, 0o644, dir_fd=cur)
    os.write(f, b"bottom\n")
    os.close(f)
    os.close(cur)
    # 2) A path just under PATH_MAX (4096 bytes including the final NUL).
    long_dir = os.path.join(d, b"long")
    os.mkdir(long_dir)
    path = os.path.abspath(long_dir)
    cur = os.open(long_dir, os.O_RDONLY | os.O_DIRECTORY)
    part = b"p" * 200
    while len(path) + 1 + len(part) + 1 + len(b"end.txt") < 4095:
        os.mkdir(part, dir_fd=cur)
        nxt = os.open(part, os.O_RDONLY | os.O_DIRECTORY, dir_fd=cur)
        os.close(cur)
        cur = nxt
        path = path + b"/" + part
    f = os.open(b"end.txt", os.O_WRONLY | os.O_CREAT, 0o644, dir_fd=cur)
    os.write(f, b"end\n")
    os.close(f)
    os.close(cur)
    # 3) A symlink loop: loop/a -> .. (a folder that contains itself).
    loop = os.path.join(d, b"loop")
    os.mkdir(loop)
    os.symlink(b"..", os.path.join(loop, b"up"))
    os.symlink(b"self", os.path.join(loop, b"self"))  # a link pointing at itself
    os.close(top)


ODD_NAMES = [
    b"invalid-\xff\xfe-utf8.txt",
    b"invalid-\xff\xfd-utf8.txt",       # differs from the one above only in an invalid byte
    b"latin1-caf\xe9.txt",
    b"line\nbreak.txt",
    b"tab\there.txt",
    b"trailing-space.txt ",
    b" leading-space.txt",
    b"-starts-with-dash.txt",
    b"--help",
    b"nfc-caf\xc3\xa9.txt",              # é as one code point
    b"nfc-cafe\xcc\x81.txt",             # e + combining accent (NFD twin of the above)
    b"emoji-\xf0\x9f\x93\x81.txt",
    b"quote'and\"double.txt",
    b"back\\slash.txt",
    b"star*question?.txt",
    b"dollar$HOME.txt",
    b"x" * 251 + b".txt",               # 255 bytes, the longest name ext4 allows
    "日".encode() * 85,                  # 255 bytes of multi-byte characters
    b".hidden",
    b"CASE.txt",
    b"case.txt",
]


def odd(d: bytes) -> None:
    os.makedirs(d, exist_ok=True)
    for name in ODD_NAMES:
        write_file(os.path.join(d, name), name, 64)
    sub = os.path.join(d, b"folder-\xff-invalid")
    os.mkdir(sub)
    write_file(os.path.join(sub, b"inside.txt"), b"inside.txt", 16)
    # Special files.
    os.mkfifo(os.path.join(d, b"fifo"))
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock_path = os.path.join(d, b"socket")
    try:
        s.bind(sock_path)
    except OSError as e:  # a path over 108 bytes cannot be bound
        print(f"socket skipped: {e}", file=sys.stderr)
    s.close()
    os.symlink(b"/dev/null", os.path.join(d, b"link-to-device"))
    os.symlink(b"fifo", os.path.join(d, b"link-to-fifo"))
    os.symlink(b"does-not-exist", os.path.join(d, b"dangling-link"))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("kind", choices=["flat", "tree", "deep", "odd"])
    ap.add_argument("dir")
    ap.add_argument("count", nargs="?", type=int, default=0)
    ap.add_argument("--size", type=int, default=0)
    ap.add_argument("--per-dir", type=int, default=1000)
    ap.add_argument("--depth", type=int, default=1000)
    a = ap.parse_args()
    d = os.fsencode(a.dir)
    if os.path.exists(d) and os.listdir(d):
        sys.exit(f"{a.dir}: not empty")
    if a.kind == "flat":
        flat(d, a.count, a.size)
    elif a.kind == "tree":
        tree(d, a.count, a.per_dir, a.size)
    elif a.kind == "deep":
        deep(d, a.depth)
    else:
        odd(d)


if __name__ == "__main__":
    main()
