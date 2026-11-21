#!/bin/bash
# Big files (roadmap 0.8.16): one real file copied to a throwaway SSH server on
# 127.0.0.1 and back through the program's own copy code: the whole file, a
# copy canceled in the middle and continued, and a copy whose process is killed
# (SIGKILL) in the middle and continued. Every copy is compared with the source
# by SHA-256. Measured: speed, the remaining-time estimate the transfer queue
# shows, memory and open files (crates/rolid-ui/src/bench_big.rs).
#
#   scripts/bench/big.sh [-g GIB] [-s STEPS] BASE [RESULTS.tsv]
#
# BASE   an empty or missing folder on a real disk with about 3.2 × GIB free
#        (source, the copy on the server and one more copy at a time).
#        Everything is created inside BASE and deleted at the end.
# GIB    size of the file in GiB (default 120).
# STEPS  comma-separated subset of: up,upkill,down,downkill,ref (default: all).
#
# Needs openssl (fast pseudo-random data), sha256sum and OpenSSH (sshd, sftp,
# ssh-keygen). Nothing is written to ~/.ssh. Processes are stopped by the PIDs
# they were started with.
set -euo pipefail
GIB=120
STEPS=up,upkill,down,downkill,ref
while getopts "g:s:" o; do
  case $o in g) GIB=$OPTARG ;; s) STEPS=$OPTARG ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 1 ] || { sed -n '2,19p' "$0"; exit 2; }
BASE=$(realpath -m "$1")
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
OUT=$(realpath -m "${2:-$BASE.tsv}")
CARGO=${CARGO:-cargo}
SIZE=$((GIB * 1024 * 1024 * 1024))
has() { [[ ",$STEPS," == *",$1,"* ]]; }

if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$BASE"
[ "$(stat -f -c %T "$BASE")" = tmpfs ] && { echo "$BASE is on tmpfs: use a real disk" >&2; exit 1; }
free_before=$(df -B1 --output=avail "$BASE" | tail -1)
need=$((SIZE / 10 * 32))
[ "$free_before" -gt "$need" ] || { echo "not enough space in $BASE: $free_before B free, $need B needed" >&2; exit 1; }
: >"$OUT"
note() { echo "== $(date +%T) $*" >&2; }
bench() { echo -e "BENCH\t$1\t$2\t$3" | tee -a "$OUT"; }
miner() { systemctl is-active --quiet moneroocean_miner.service 2>/dev/null && echo 1 || echo 0; }

SSHD_PID= JOB_PID=
cleanup() {
  [ -n "$JOB_PID" ] && kill -KILL "$JOB_PID" 2>/dev/null || true
  [ -n "$SSHD_PID" ] && kill "$SSHD_PID" 2>/dev/null || true
  [ -n "$SSHD_PID" ] && wait "$SSHD_PID" 2>/dev/null || true
  rm -rf "$BASE"
  free_after=$(df -B1 --output=avail "$(dirname "$BASE")" | tail -1)
  echo "free space before: $free_before, after: $free_after (difference $(( (free_after - free_before) / 1048576 )) MB)" >&2
}
trap cleanup EXIT

mkdir -p "$BASE/local" "$BASE/server" "$BASE/ssh"
bench big.miner_at_start "$(miner)" bool
bench big.size "$SIZE" B

note "building"
TEST_BIN=$(cd "$ROOT" && "$CARGO" test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
  python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')

note "SSH server"
S="$BASE/ssh"
ssh-keygen -q -t ed25519 -N "" -f "$S/host"
ssh-keygen -q -t ed25519 -N "" -f "$S/client"
cp "$S/client.pub" "$S/authorized_keys"
PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
cat >"$S/sshd_config" <<EOF
Port $PORT
ListenAddress 127.0.0.1
HostKey $S/host
AuthorizedKeysFile $S/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
PidFile $S/sshd.pid
Subsystem sftp internal-sftp
AllowUsers $(id -un)
LogLevel ERROR
EOF
echo "[127.0.0.1]:$PORT $(cut -d' ' -f1,2 "$S/host.pub")" >"$S/known_hosts"
/usr/sbin/sshd -D -e -f "$S/sshd_config" 2>"$S/sshd.log" &
SSHD_PID=$!
for _ in $(seq 50); do (exec 3<>/dev/tcp/127.0.0.1/"$PORT") 2>/dev/null && break; sleep 0.1; done
export ROLIDSCP_BIG_SSH="$PORT|$(id -un)|$S/client|$S/known_hosts" ROLIDSCP_BENCH_OUT="$OUT"

SRC="$BASE/local/big.bin"
note "writing $GIB GiB of pseudo-random data"
t0=$(date +%s)
# (openssl ends with SIGPIPE when head has enough: not an error.)
want=$( { openssl enc -aes-128-ctr -K 00112233445566778899aabbccddeeff -iv 0 -nosalt -in /dev/zero 2>/dev/null || true; } |
  head -c "$SIZE" | tee "$SRC" | sha256sum | cut -d' ' -f1)
bench big.write_source_s "$(( $(date +%s) - t0 ))" s

# check FILE NAME: the SHA-256 of FILE against the source's.
check() {
  local t0 got
  t0=$(date +%s)
  got=$(sha256sum "$1" | cut -d' ' -f1)
  bench "big.$2.sha256_equal" "$([ "$got" = "$want" ] && echo 1 || echo 0)" bool
  bench "big.$2.sha256_s" "$(( $(date +%s) - t0 ))" s
  [ "$got" = "$want" ] || note "CHECKSUM DIFFERS: $1"
}
# stage TAG WAY STAGE SRC DST_DIR NAME: one stage in the foreground.
stage() {
  note "$1"
  ROLIDSCP_BIG_TAG="big.$1" ROLIDSCP_BIG_DIR=$2 ROLIDSCP_BIG_STAGE=$3 ROLIDSCP_BIG_SRC=$4 ROLIDSCP_BIG_DST=$5 ROLIDSCP_BIG_NAME=$6 \
    "$TEST_BIN" --ignored --exact bench_big::bench_big --test-threads=1 --nocapture 2>&1 |
    grep -v "^BENCH" | grep -E -A3 "panicked|result:|SKIP" >&2 || true
}
# killed TAG WAY SRC DST_DIR NAME: a copy killed with SIGKILL at half of the
# file, then continued by a new process.
killed() {
  note "$1: copy, killed at 50 %"
  local part="$4/$5.filepart" t0
  t0=$(date +%s)
  ROLIDSCP_BIG_TAG="big.$1.first" ROLIDSCP_BIG_DIR=$2 ROLIDSCP_BIG_STAGE=copy ROLIDSCP_BIG_SRC=$3 ROLIDSCP_BIG_DST=$4 ROLIDSCP_BIG_NAME=$5 \
    "$TEST_BIN" --ignored --exact bench_big::bench_big --test-threads=1 --nocapture >/dev/null 2>&1 &
  JOB_PID=$!
  while kill -0 "$JOB_PID" 2>/dev/null; do
    if [ "$(stat -c %s "$part" 2>/dev/null || echo 0)" -ge $((SIZE / 2)) ]; then
      kill -KILL "$JOB_PID"
      break
    fi
    sleep 0.2
  done
  wait "$JOB_PID" 2>/dev/null || true
  JOB_PID=
  bench "big.$1.first.seconds" "$(( $(date +%s) - t0 ))" s
  bench "big.$1.first.filepart" "$(stat -c %s "$part" 2>/dev/null || echo 0)" B
  stage "$1.resume" "$2" resume "$3" "$4" "$5"
}

if has up; then
  stage up up cancel "$SRC" "$BASE/server" big.bin
  check "$BASE/server/big.bin" up
fi
if has upkill; then
  killed upkill up "$SRC" "$BASE/server" big-k.bin
  check "$BASE/server/big-k.bin" upkill
  rm -f "$BASE/server/big-k.bin"
fi
# Downloads read the copy on the server (or the source, when "up" was not run).
REMOTE="$BASE/server/big.bin"
[ -f "$REMOTE" ] || REMOTE=$SRC
if has down; then
  stage down down cancel "$REMOTE" "$BASE/local" down.bin
  check "$BASE/local/down.bin" down
  rm -f "$BASE/local/down.bin"
fi
if has downkill; then
  killed downkill down "$REMOTE" "$BASE/local" down-k.bin
  check "$BASE/local/down-k.bin" downkill
  rm -f "$BASE/local/down-k.bin"
fi
if has ref; then
  OPTS=(-i "$S/client" -o UserKnownHostsFile="$S/known_hosts" -o StrictHostKeyChecking=yes -o BatchMode=yes
        -o IdentitiesOnly=yes -o IdentityAgent=none -o UpdateHostKeys=no -F /dev/null)
  SFTP() { sftp -q -b - "${OPTS[@]}" -P "$PORT" "$(id -un)@127.0.0.1" >/dev/null; }
  note "reference: OpenSSH sftp put"
  t0=$(date +%s); SFTP <<<"put $SRC $BASE/server/ref.bin"; bench big.ref.sftp_put_s "$(( $(date +%s) - t0 ))" s
  rm -f "$BASE/server/ref.bin"
  note "reference: OpenSSH sftp get"
  t0=$(date +%s); SFTP <<<"get $REMOTE $BASE/local/ref.bin"; bench big.ref.sftp_get_s "$(( $(date +%s) - t0 ))" s
  rm -f "$BASE/local/ref.bin"
  note "reference: local copy on the same disk"
  t0=$(date +%s); cat "$SRC" >"$BASE/local/ref.bin"; sync "$BASE/local/ref.bin"; bench big.ref.local_copy_s "$(( $(date +%s) - t0 ))" s
  rm -f "$BASE/local/ref.bin"
fi
bench big.miner_at_end "$(miner)" bool
note "done"
