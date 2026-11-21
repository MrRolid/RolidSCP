#!/bin/bash
# Disk space (roadmap 0.8.16), crates/rolid-ui/src/bench_space.rs:
# - a full disk in the middle of a transfer, on both sides: a small tmpfs
#   mounted in a user namespace of its own (no root needed), reached through
#   /proc/PID/root by the test and by a throwaway SSH server on 127.0.0.1;
# - sparse files: a file of GIB GiB that is almost all holes, copied on this
#   computer, to the server and back.
#
#   scripts/bench/space.sh [-g GIB] BASE [RESULTS.tsv]
#
# BASE  an empty or missing folder on a real disk with about 2 × GIB + 0.2 GB
#       free (default GIB 2); the tmpfs takes 128 MB of memory. Everything is
#       deleted at the end.
#
# Needs unshare (util-linux) with user namespaces allowed, OpenSSH (sshd,
# ssh-keygen). Nothing is written to ~/.ssh. Processes are stopped by the
# PIDs they were started with.
set -euo pipefail
GIB=2
while getopts "g:" o; do
  case $o in g) GIB=$OPTARG ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 1 ] || { sed -n '2,17p' "$0"; exit 2; }
BASE=$(realpath -m "$1")
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
OUT=$(realpath -m "${2:-$BASE.tsv}")
CARGO=${CARGO:-cargo}
if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$BASE/mnt" "$BASE/src" "$BASE/sparse"
[ "$(stat -f -c %T "$BASE")" = tmpfs ] && { echo "$BASE is on tmpfs: use a real disk" >&2; exit 1; }
: >"$OUT"
note() { echo "== $(date +%T) $*" >&2; }
miner() { systemctl is-active --quiet moneroocean_miner.service 2>/dev/null && echo 1 || echo 0; }

SSHD_PID= NS_PID=
cleanup() {
  [ -n "$SSHD_PID" ] && kill "$SSHD_PID" 2>/dev/null || true
  [ -n "$SSHD_PID" ] && wait "$SSHD_PID" 2>/dev/null || true
  [ -n "$NS_PID" ] && kill "$NS_PID" 2>/dev/null || true
  [ -n "$NS_PID" ] && wait "$NS_PID" 2>/dev/null || true
  rm -rf "$BASE"
}
trap cleanup EXIT
echo -e "BENCH\tfull.miner_at_start\t$(miner)\tbool" | tee -a "$OUT"

note "building"
TEST_BIN=$(cd "$ROOT" && "$CARGO" test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
  python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')

note "a 128 MB tmpfs in a user namespace"
unshare -rm sh -c 'mount -t tmpfs -o size=128m tmpfs "$1" && exec sleep 100000' sh "$BASE/mnt" &
NS_PID=$!
FULL=/proc/$NS_PID/root$BASE/mnt
for _ in $(seq 50); do [ "$(stat -f -c %T "$FULL" 2>/dev/null)" = tmpfs ] && break; sleep 0.1; done
[ "$(stat -f -c %T "$FULL")" = tmpfs ] || { echo "the tmpfs is not there" >&2; exit 1; }
mkdir "$FULL/t"

. "$HERE/sshd-lib.sh"
start_sshd "$BASE/ssh"

note "full disk: upload and download"
ROLIDSCP_FULL_DIR="$FULL/t" ROLIDSCP_FULL_SRC="$BASE/src" ROLIDSCP_BENCH_OUT="$OUT" \
  "$TEST_BIN" --ignored --exact bench_space::bench_full --test-threads=1 --nocapture 2>&1 |
  grep -E "^FULLMSG|panicked|result:" -A3 | tee -a "$OUT.log" >&2 || true
note "sparse files ($GIB GiB)"
ROLIDSCP_SPARSE_DIR="$BASE/sparse" ROLIDSCP_SPARSE_GIB="$GIB" ROLIDSCP_BENCH_OUT="$OUT" \
  "$TEST_BIN" --ignored --exact bench_space::bench_sparse --test-threads=1 --nocapture 2>&1 |
  grep -E "panicked|result:" -A3 | tee -a "$OUT.log" >&2 || true
echo -e "BENCH\tfull.miner_at_end\t$(miner)\tbool" | tee -a "$OUT"
note "done"
