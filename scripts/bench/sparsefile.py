#!/usr/bin/env python3
"""A big test file that takes little disk space and still catches misplaced data.

    sparsefile.py PATH GIB [SEED]

Writes a sparse file of GIB GiB: every 256 KiB starts with 4 KiB of data that
depends only on SEED and the block's number (SHAKE-256), the rest are holes
(zeros). It takes 1/64 of its size on the disk (about 1.8 GB for 110 GiB), so
a copy of it fits where the source would not, and the same file can be made
again on another computer. A block that went to a wrong place, was lost or
written twice changes the SHA-256 of the copy. Prints the SHA-256 of the file.
"""
import hashlib
import os
import sys

STEP = 256 * 1024
DATA = 4096


def main() -> None:
    if len(sys.argv) not in (3, 4):
        sys.exit(__doc__)
    path, gib = sys.argv[1], float(sys.argv[2])
    seed = (sys.argv[3] if len(sys.argv) == 4 else "rolidscp").encode()
    size = int(gib * 1024**3)
    h = hashlib.sha256()
    zeros = bytes(STEP - DATA)
    with open(path, "wb") as f:
        f.truncate(size)
        for off in range(0, size, STEP):
            block = hashlib.shake_256(seed + off.to_bytes(8, "little")).digest(DATA)
            n = min(DATA, size - off)
            f.seek(off)
            f.write(block[:n])
            h.update(block[:n])
            rest = min(STEP, size - off) - n
            h.update(zeros[:rest] if rest == len(zeros) else bytes(rest))
    print(h.hexdigest())


if __name__ == "__main__":
    main()
