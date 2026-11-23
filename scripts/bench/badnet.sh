#!/bin/bash
# A bad line (roadmap 0.8.17): transfers through the program's copy code to a
# throwaway SSH server over a network with latency, jitter, packet loss, a
# changing speed, repeated outages and a frozen connection; every file is
# compared by SHA-256 (crates/rolid-ui/src/bench_net.rs).
#
#   scripts/bench/badnet.sh [-m MB] [-s SCENARIOS] [-r] BASE [RESULTS.tsv]
#
# BASE       an empty or missing folder on a real disk (about 3 × MB used).
# MB         size of the big file (default 32); 200 small files go with it.
# SCENARIOS  comma-separated subset (default: all of SCENARIOS below; the
#            ones in EXTRA run only when named here).
# -r         also move the big file with OpenSSH's sftp over the same line
#            (net.<scenario>.sftp_up_ms / sftp_down_ms), for comparison.
# ROLIDSCP_NET_SMALL=0 leaves out the small files.
#
# No root needed: everything runs in a private user and network namespace
# (unshare -rn). In it this script is "root" only for that namespace and
# sets the line on its loopback with `tc netem`; the SSH server runs in a
# nested namespace as the normal user. Nothing is written to ~/.ssh.
# Processes are stopped by the PIDs they were started with.
set -euo pipefail
MB=32
ONLY=
REF=
while getopts "m:s:r" o; do
  case $o in m) MB=$OPTARG ;; s) ONLY=$OPTARG ;; r) REF=1 ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 1 ] || { sed -n '2,20p' "$0"; exit 2; }
BASE=$(realpath -m "$1")
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
OUT=$(realpath -m "${2:-$BASE.tsv}")
CARGO=${CARGO:-cargo}
TC=/usr/sbin/tc

if [ "${BADNET_INSIDE:-}" != 1 ]; then
  # ---- Outside: build, then re-run this script in the namespaces. ----
  if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
  mkdir -p "$BASE"
  free_before=$(df -B1 --output=avail "$BASE" | tail -1)
  echo "== $(date +%T) building" >&2
  TEST_BIN=$(cd "$ROOT" && "$CARGO" test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
    python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')
  : >"$OUT"
  set +e
  BADNET_INSIDE=1 TEST_BIN="$TEST_BIN" REAL_UID=$(id -u) REAL_GID=$(id -g) \
    unshare -rn --fork --pid --mount-proc bash "$0" -m "$MB" ${ONLY:+-s "$ONLY"} ${REF:+-r} "$BASE" "$OUT"
  rc=$?
  set -e
  rm -rf "$BASE"
  free_after=$(df -B1 --output=avail "$(dirname "$BASE")" | tail -1)
  echo "free space before: $free_before, after: $free_after (difference $(( (free_after - free_before) / 1048576 )) MB)" >&2
  exit $rc
fi

# ---- Inside the namespaces (root of this namespace only). ----
note() { echo "== $(date +%T) $*" >&2; }
# A real line: packets of 1500 bytes (the loopback's 64 KiB would let the
# emulator queue 64 MB, minutes at a low speed, which no real network does).
ip link set lo mtu 1500 up
mkdir -p "$BASE/w"
# The SSH server as the normal user (sshd refuses to run as a mapped root).
unshare --map-user="$REAL_UID" --map-group="$REAL_GID" -U bash -c '
  . "$1/sshd-lib.sh"; start_sshd "$2/ssh"; echo "$ROLIDSCP_BIG_SSH" >"$2/ssh.env.tmp"; mv "$2/ssh.env.tmp" "$2/ssh.env"; wait "$SSHD_PID"' _ "$HERE" "$BASE" &
SSHD_NS=$!
for _ in $(seq 100); do [ -e "$BASE/ssh.env" ] && break; sleep 0.1; done
[ -e "$BASE/ssh.env" ] || { echo "the SSH server did not start" >&2; exit 1; }
export ROLIDSCP_BIG_SSH=$(cat "$BASE/ssh.env") ROLIDSCP_NET_DIR="$BASE/w" ROLIDSCP_NET_MB=$MB

# With a speed limit the queue holds 300 packets (450 kB: under a second at
# 4 Mbit/s, 7 s at 512 kbit/s), like a router with a big buffer.
line() {
  case " $* " in *" rate "*) $TC qdisc replace dev lo root netem limit 300 "$@" ;; *) $TC qdisc replace dev lo root netem "$@" ;; esac
}
clear_line() { $TC qdisc del dev lo root 2>/dev/null || true; }

# Disturbers: change the line while a transfer runs (stopped by their PID).
rates() { while :; do for r in 20mbit 2mbit 512kbit 50mbit 1mbit; do line delay 20ms rate "$r"; sleep 5; done; done; }
# The outages come early and the line is slow (4 Mbit/s, about 70 s a way for
# 32 MB), so every transfer meets several of them.
SLOW="delay 30ms 10ms rate 4mbit"
# Short: the connection survives them (TCP sends again).
outages() { while :; do sleep 6; line loss 100%; sleep 5; line $SLOW; done; }
# Long (30 s), each followed by a minute of a working line: the program must
# get over every one of them (keep the connection or connect again) and finish.
long_outages() { sleep 8; while :; do line loss 100%; sleep 30; line $SLOW; sleep 60; done; }
# Not in the default set: 30 s down, 8 s up, again and again. No TCP connection
# gets over that (after 30 s without answers TCP waits about 25 s before it
# sends again, by then the line is down again); OpenSSH's sftp gives up too.
flapping() { while :; do sleep 8; line loss 100%; sleep 30; line $SLOW; done; }
# Frozen for longer than the keepalive limit: the program must notice, reconnect and resume.
frozen() { sleep 8; line loss 100%; sleep 75; line $SLOW; sleep 100000; }
everything() {
  while :; do
    line delay 80ms 30ms loss 1% rate 8mbit; sleep 8
    line delay 150ms 60ms loss 3% rate 1mbit; sleep 8
    line loss 100%; sleep 6
    line delay 40ms 10ms rate 30mbit; sleep 8
  done
}

# name | starting line (netem arguments, "-" = none) | disturber ("-" = none)
SCENARIOS=(
  "clean|-|-"
  "latency|delay 100ms 20ms distribution normal|-"
  "jitter|delay 60ms 50ms|-"
  "loss2|delay 20ms loss 2%|-"
  "loss5|delay 20ms loss 5%|-"
  "rates|delay 20ms rate 20mbit|rates"
  "outages|delay 30ms 10ms rate 4mbit|outages"
  "long_outages|delay 30ms 10ms rate 4mbit|long_outages"
  "frozen|delay 30ms 10ms rate 4mbit|frozen"
  "everything|delay 80ms 30ms loss 1% rate 8mbit|everything"
)
# Run only when named with -s.
EXTRA=(
  "flapping|delay 30ms 10ms rate 4mbit|flapping"
  # A server far away without jitter (jitter reorders packets, which makes
  # every TCP transfer vary a lot from run to run).
  "far|delay 100ms|-"
  # The same with room for 100 000 packets on the way (the emulator's default
  # of 1000 holds every program to about 3.5 MB/s at 100 ms).
  "far_fast|limit 100000 delay 100ms|-"
)
# OpenSSH's sftp with the same file over the same line: prints milliseconds.
# Its own settings only (no ~/.ssh: -F /dev/null), the host key from the file.
sftp_ref() {
  local port user key kh t0
  IFS='|' read -r port user key kh <<<"$ROLIDSCP_BIG_SSH"
  t0=$(date +%s%N)
  timeout 900 sftp -q -F /dev/null -b - -i "$key" -P "$port" -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes \
    -o UserKnownHostsFile="$kh" -o GlobalKnownHostsFile=/dev/null -o ServerAliveInterval=15 \
    "$user@127.0.0.1" <<<"$1" >/dev/null 2>>"$BASE/sftp.err" || { echo -1; return; }
  echo $(( ($(date +%s%N) - t0) / 1000000 ))
}

fail=0
ALL=("${SCENARIOS[@]}")
[ -n "$ONLY" ] && ALL+=("${EXTRA[@]}")
for s in "${ALL[@]}"; do
  IFS='|' read -r name start dist <<<"$s"
  if [ -n "$ONLY" ] && [[ ",$ONLY," != *",$name,"* ]]; then continue; fi
  clear_line
  # shellcheck disable=SC2086
  [ "$start" = - ] || line $start
  note "$name: ${start} ${dist}"
  DPID=
  if [ "$dist" != - ]; then "$dist" & DPID=$!; fi
  t0=$(date +%s)
  set +e
  ROLIDSCP_NET_TAG=$name timeout 1800 "$TEST_BIN" --ignored --exact bench_net::bench_net --test-threads=1 --nocapture >"$BASE/$name.log" 2>&1
  rc=$?
  set -e
  [ -n "$DPID" ] && { kill "$DPID" 2>/dev/null || true; wait "$DPID" 2>/dev/null || true; }
  if [ -n "$REF" ]; then
    # The same line again from its start, with its disturber.
    clear_line
    # shellcheck disable=SC2086
    [ "$start" = - ] || line $start
    DPID=
    if [ "$dist" != - ]; then "$dist" & DPID=$!; fi
    big=$(ls "$BASE"/w/src-*/data/big.bin | head -1)
    rm -f "$BASE/w/ref-up.bin" "$BASE/w/ref-down.bin"
    up_ms=$(sftp_ref "put $big $BASE/w/ref-up.bin")
    down_ms=$(sftp_ref "get $BASE/w/ref-up.bin $BASE/w/ref-down.bin")
    [ -n "$DPID" ] && { kill "$DPID" 2>/dev/null || true; wait "$DPID" 2>/dev/null || true; }
    cmp -s "$big" "$BASE/w/ref-down.bin" && same=1 || same=0
    rm -f "$BASE/w/ref-up.bin" "$BASE/w/ref-down.bin"
    echo -e "BENCH\tnet.$name.sftp_up_ms\t$up_ms\tms\nBENCH\tnet.$name.sftp_down_ms\t$down_ms\tms\nBENCH\tnet.$name.sftp_same\t$same\tbool" | tee -a "$OUT"
  fi
  clear_line
  cp "$BASE/$name.log" "${OUT%.tsv}-$name.log"
  grep "^BENCH" "$BASE/$name.log" | tee -a "$OUT"
  echo -e "BENCH\tnet.$name.total_s\t$(( $(date +%s) - t0 ))\ts" | tee -a "$OUT"
  echo -e "BENCH\tnet.$name.passed\t$([ $rc = 0 ] && echo 1 || echo 0)\tbool" | tee -a "$OUT"
  if [ $rc != 0 ]; then
    fail=1
    { grep -E "panicked|connect:" -A3 "$BASE/$name.log" || tail -20 "$BASE/$name.log"; } | head -30 | tee -a "$OUT.log" >&2
  fi
done
kill "$SSHD_NS" 2>/dev/null || true
note "done"
exit $fail
