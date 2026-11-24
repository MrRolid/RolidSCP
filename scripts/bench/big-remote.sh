#!/bin/bash
# Big files to another computer over a real network (roadmap 0.8.19): a
# sparse file of over 100 GB (scripts/bench/sparsefile.py, 1/64 of its size on
# this disk) copied to an existing SSH server through the program's own copy
# code (crates/rolid-ui/src/bench_big.rs): canceled at half and continued with
# "Resume", killed (SIGKILL) at half and continued by a new process, and
# OpenSSH's sftp as a reference. The copy is written whole on the server and
# compared there by SHA-256, then deleted. Measured: speed, the remaining-time
# estimate the transfer queue shows, memory and open files.
#
#   scripts/bench/big-remote.sh [-g GIB] [-s STEPS] [-b TEST_BIN] BASE REMOTE_DIR [RESULTS.tsv]
#
# BASE        an empty or missing folder here for the sparse source (deleted
#             at the end)
# REMOTE_DIR  a folder on the server that does not exist yet (made, deleted
#             at the end); it needs GIB GiB free
# GIB         size of the file in GiB (default 110)
# STEPS       comma-separated subset of: up,upkill,ref,eta,etap (default:
#             up,upkill,ref); eta copies the whole file once at full speed,
#             etap once with the speed profile BIG_PROFILE (see bench_big.rs,
#             ROLIDSCP_BIG_PROFILE), both for the remaining-time estimate
#             (the bytes once a second go to BIG_TRACE, a folder, if set)
# TEST_BIN    the test binary of the rolidscp crate (default: built with cargo)
#
# The server, from the environment (nothing is written to ~/.ssh; the host
# key must be in BIG_KNOWN_HOSTS already):
#   BIG_HOST, BIG_PORT (default 22), BIG_USER, BIG_KEY, BIG_KNOWN_HOSTS
#
# Processes are stopped by the PIDs they were started with.
set -euo pipefail
GIB=110
STEPS=up,upkill,ref
TEST_BIN=
while getopts "g:s:b:" o; do
  case $o in g) GIB=$OPTARG ;; s) STEPS=$OPTARG ;; b) TEST_BIN=$(realpath "$OPTARG") ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 2 ] || { sed -n '2,28p' "$0"; exit 2; }
: "${BIG_HOST:?}" "${BIG_USER:?}" "${BIG_KEY:?}" "${BIG_KNOWN_HOSTS:?}"
BIG_PORT=${BIG_PORT:-22}
BASE=$(realpath -m "$1")
RDIR=$2
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
OUT=$(realpath -m "${3:-$BASE.tsv}")
SIZE=$((GIB * 1024 * 1024 * 1024))
has() { [[ ",$STEPS," == *",$1,"* ]]; }
case $RDIR in /*) ;; *) echo "REMOTE_DIR must be absolute" >&2; exit 2 ;; esac

if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$BASE"
: >"$OUT"
note() { echo "== $(date +%T) $*" >&2; }
bench() { echo -e "BENCH\t$1\t$2\t$3" | tee -a "$OUT"; }

OPTS=(-i "$BIG_KEY" -o UserKnownHostsFile="$BIG_KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes
      -o IdentitiesOnly=yes -o IdentityAgent=none -o UpdateHostKeys=no -F /dev/null
      -o ControlMaster=auto -o ControlPath="$BASE/cm" -o ControlPersist=120)
R() { ssh "${OPTS[@]}" -p "$BIG_PORT" "$BIG_USER@$BIG_HOST" "$@"; }

JOB_PID=
cleanup() {
  [ -n "$JOB_PID" ] && kill -KILL "$JOB_PID" 2>/dev/null || true
  R "rm -rf -- '$RDIR'" 2>/dev/null || true
  R -O exit 2>/dev/null || true
  rm -rf "$BASE"
}
trap cleanup EXIT

R "test ! -e '$RDIR' && mkdir -p -- '$RDIR'" || { echo "$RDIR exists on the server or cannot be made" >&2; exit 1; }
free=$(R "df -B1 --output=avail '$RDIR' | tail -1")
[ "$free" -gt $((SIZE / 10 * 11)) ] || { echo "not enough space on the server: $free B" >&2; exit 1; }
bench big_remote.size "$SIZE" B
bench big_remote.server_free_before "$free" B

if [ -z "$TEST_BIN" ]; then
  note "building"
  TEST_BIN=$(cd "$ROOT" && cargo test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
    python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')
fi
[ -n "${BIG_TRACE:-}" ] && mkdir -p "$BIG_TRACE" && export ROLIDSCP_BIG_TRACE=$(realpath "$BIG_TRACE")
export ROLIDSCP_BIG_HOST=$BIG_HOST ROLIDSCP_BIG_SSH="$BIG_PORT|$BIG_USER|$BIG_KEY|$BIG_KNOWN_HOSTS" ROLIDSCP_BENCH_OUT="$OUT"

SRC="$BASE/big.bin"
note "writing a sparse file of $GIB GiB"
want=$(python3 -I "$HERE/sparsefile.py" "$SRC" "$GIB")
bench big_remote.source_on_disk "$(du -B1 "$SRC" | cut -f1)" B

# check NAME TAG: the SHA-256 of the copy on the server against the source's.
check() {
  local t0 got
  t0=$(date +%s)
  got=$(R "sha256sum -- '$RDIR/$1'" | cut -d' ' -f1)
  bench "big_remote.$2.sha256_equal" "$([ "$got" = "$want" ] && echo 1 || echo 0)" bool
  bench "big_remote.$2.sha256_s" "$(( $(date +%s) - t0 ))" s
  [ "$got" = "$want" ] || note "CHECKSUM DIFFERS: $1"
  R "rm -f -- '$RDIR/$1'"
}
stage() {
  note "$1"
  ROLIDSCP_BIG_TAG="big_remote.$1" ROLIDSCP_BIG_DIR=up ROLIDSCP_BIG_STAGE=$2 ROLIDSCP_BIG_SRC=$SRC ROLIDSCP_BIG_DST=$RDIR ROLIDSCP_BIG_NAME=$3 \
    "$TEST_BIN" --ignored --exact bench_big::bench_big --test-threads=1 --nocapture 2>&1 |
    grep -v "^BENCH" | grep -E -A3 "panicked|result:|SKIP" >&2 || true
}

if has up; then
  stage up cancel big.bin
  check big.bin up
fi
if has upkill; then
  note "upkill: copy, killed at 50 %"
  t0=$(date +%s)
  ROLIDSCP_BIG_TAG="big_remote.upkill.first" ROLIDSCP_BIG_DIR=up ROLIDSCP_BIG_STAGE=copy ROLIDSCP_BIG_SRC=$SRC ROLIDSCP_BIG_DST=$RDIR ROLIDSCP_BIG_NAME=big-k.bin \
    "$TEST_BIN" --ignored --exact bench_big::bench_big --test-threads=1 --nocapture >/dev/null 2>&1 &
  JOB_PID=$!
  while kill -0 "$JOB_PID" 2>/dev/null; do
    if [ "$(R "stat -c %s -- '$RDIR/big-k.bin.filepart' 2>/dev/null || echo 0")" -ge $((SIZE / 2)) ]; then
      kill -KILL "$JOB_PID"
      break
    fi
    sleep 2
  done
  wait "$JOB_PID" 2>/dev/null || true
  JOB_PID=
  bench big_remote.upkill.first.seconds "$(( $(date +%s) - t0 ))" s
  bench big_remote.upkill.first.filepart "$(R "stat -c %s -- '$RDIR/big-k.bin.filepart' 2>/dev/null || echo 0")" B
  stage upkill.resume resume big-k.bin
  check big-k.bin upkill
fi
if has eta; then
  stage eta copy big-e.bin
  check big-e.bin eta
fi
if has etap; then
  : "${BIG_PROFILE:?etap needs BIG_PROFILE}"
  ROLIDSCP_BIG_PROFILE=$BIG_PROFILE stage etap copy big-p.bin
  check big-p.bin etap
fi
if has ref; then
  note "reference: OpenSSH sftp put"
  t0=$(date +%s)
  sftp -q -b - "${OPTS[@]}" -P "$BIG_PORT" "$BIG_USER@$BIG_HOST" >/dev/null <<<"put $SRC $RDIR/ref.bin"
  bench big_remote.ref.sftp_put_s "$(( $(date +%s) - t0 ))" s
  check ref.bin ref
fi
note "done"
