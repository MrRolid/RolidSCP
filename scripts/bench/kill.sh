#!/bin/bash
# The program killed (SIGKILL) at a random moment of an operation (roadmap
# 0.8.16): copying and moving to and from a throwaway SSH server on 127.0.0.1,
# uploading an edited file (F4), packing, extracting, saving the settings and
# the password vault. After each kill the state is checked (nothing lost,
# nothing damaged under its real name, settings and vault readable), the work
# is finished the way the program does it and compared byte by byte
# (crates/rolid-ui/src/bench_kill.rs).
#
#   scripts/bench/kill.sh [-n ROUNDS] BASE [RESULTS.tsv]
#
# BASE    an empty or missing folder on a real disk (about 400 MB are used).
# ROUNDS  kills in all, spread over the operations (default 200).
#
# Needs OpenSSH (sshd, ssh-keygen). Nothing is written to ~/.ssh. Processes
# are stopped by the PIDs they were started with.
set -euo pipefail
ROUNDS=200
while getopts "n:" o; do
  case $o in n) ROUNDS=$OPTARG ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 1 ] || { sed -n '2,16p' "$0"; exit 2; }
BASE=$(realpath -m "$1")
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
OUT=$(realpath -m "${2:-$BASE.tsv}")
CARGO=${CARGO:-cargo}
if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$BASE/w"
free_before=$(df -B1 --output=avail "$BASE" | tail -1)
: >"$OUT"
note() { echo "== $(date +%T) $*" >&2; }
bench() { echo -e "BENCH\t$1\t$2\t$3" | tee -a "$OUT"; }

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

note "building"
TEST_BIN=$(cd "$ROOT" && "$CARGO" test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
  python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')
. "$HERE/sshd-lib.sh"
start_sshd "$BASE/ssh"
export ROLIDSCP_KILL_DIR="$BASE/w"
W="$BASE/w"

stage() { # OP STAGE
  ROLIDSCP_KILL_OP=$1 ROLIDSCP_KILL_STAGE=$2 "$TEST_BIN" --ignored --exact bench_kill::bench_kill --test-threads=1 --nocapture 2>&1
}
fresh_source() { # OP: a new source for a move
  case $1 in move_*) rm -rf "${W:?}/$1-from"; mkdir -p "$W/$1-from"; cp -r "$W/src/data" "$W/$1-from/" ;; esac
}
note "preparing"
stage prepare prepare | grep -E "panicked|SKIP" >&2 || true

OPS=(copy_up copy_down move_up move_down f4 pack unpack settings vault)
declare -A LONG OK FAIL FINISHED
note "how long each operation takes when it is not killed"
for op in "${OPS[@]}"; do
  case $op in
    settings | vault) LONG[$op]=1000 ;; # endless loops of saving: killed at any moment
    *)
      fresh_source "$op"
      t0=$(date +%s%N)
      stage "$op" do | grep -E "panicked" -A3 >&2 || true
      LONG[$op]=$(( ($(date +%s%N) - t0) / 1000000 ))
      stage "$op" check >/dev/null 2>&1 || { note "$op: check after a full run failed"; exit 1; }
      ;;
  esac
  bench "kill.$op.full_ms" "${LONG[$op]}" ms
  OK[$op]=0 FAIL[$op]=0 FINISHED[$op]=0
done

note "$ROUNDS kills"
for ((r = 0; r < ROUNDS; r++)); do
  op=${OPS[$((r % ${#OPS[@]}))]}
  fresh_source "$op"
  # A random moment in the operation (process start included).
  delay=$(( RANDOM * LONG[$op] * 12 / 10 / 32768 ))
  stage "$op" do >/dev/null 2>&1 &
  JOB_PID=$!
  sleep "$(printf '%d.%03d' $((delay / 1000)) $((delay % 1000)))"
  if kill -KILL "$JOB_PID" 2>/dev/null; then :; else FINISHED[$op]=$(( FINISHED[$op] + 1 )); fi
  wait "$JOB_PID" 2>/dev/null || true
  JOB_PID=
  if res=$(stage "$op" check) && line=$(grep "^KILLCHECK" <<<"$res"); then
    OK[$op]=$(( OK[$op] + 1 ))
    echo "$r $op ${delay}ms ${line#KILLCHECK }" >>"$OUT.log"
  else
    FAIL[$op]=$(( FAIL[$op] + 1 ))
    echo "$r $op ${delay}ms FAILED" >>"$OUT.log"
    grep -E "panicked" -A4 <<<"$res" | tee -a "$OUT.log" >&2
  fi
done
for op in "${OPS[@]}"; do
  bench "kill.$op.ok" "${OK[$op]}" rounds
  bench "kill.$op.failed" "${FAIL[$op]}" rounds
  bench "kill.$op.finished_before_kill" "${FINISHED[$op]}" rounds
done
note "done (details in $OUT.log)"
