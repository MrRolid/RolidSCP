#!/bin/bash
# Load test with many files (roadmap 0.8.15): generate the trees, run the
# measurements and the same work with standard tools for comparison, measure
# frame times in the real window, delete everything and print a table.
#
#   scripts/bench/measure.sh [-n FILES] [-s STEPS] BASE [RESULTS.tsv]
#
# BASE     an empty or missing folder on a real disk (not tmpfs: a million files
#          need a million inodes). Needs about FILES × 12 kB free at the peak
#          (1 000 000 files: ~12 GB). Everything is created inside BASE and
#          deleted at the end.
# FILES    files in the big folder and in the copied tree (default 1000000).
# STEPS    comma-separated subset of: list,sftp,size,search,copy,leak,deep,ref,gui
#          (default: all).
#
# Needs python3, OpenSSH (sshd, ssh, sftp, ssh-keygen); Xvfb, libX11 and libXtst
# for the window. The SSH server is a throwaway one on 127.0.0.1 with its own
# keys in BASE; nothing is written to ~/.ssh. Processes are stopped by the PIDs
# they were started with.
set -euo pipefail
N=1000000
STEPS=list,sftp,size,search,copy,leak,deep,ref,gui
while getopts "n:s:" o; do
  case $o in n) N=$OPTARG ;; s) STEPS=$OPTARG ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 1 ] || { sed -n '2,20p' "$0"; exit 2; }
BASE=$(realpath -m "$1")
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
OUT=$(realpath -m "${2:-$BASE.tsv}")
CARGO=${CARGO:-cargo}
has() { [[ ",$STEPS," == *",$1,"* ]]; }

if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$(dirname "$BASE")"
fstype=$(stat -f -c %T "$(dirname "$BASE")")
[ "$fstype" = tmpfs ] && { echo "$BASE is on tmpfs: use a real disk" >&2; exit 1; }
mkdir -p "$BASE"
free_before=$(df -B1 --output=avail "$BASE" | tail -1)
: >"$OUT"
note() { echo "== $*" >&2; }
bench() { echo -e "BENCH\t$1\t$2\t$3" | tee -a "$OUT"; }
ms_of() { local t0; t0=$(date +%s%N); "$@" >/dev/null; echo $(( ($(date +%s%N) - t0) / 1000000 )); }

SSHD_PID=
cleanup() {
  [ -n "$SSHD_PID" ] && kill "$SSHD_PID" 2>/dev/null || true
  [ -n "$SSHD_PID" ] && wait "$SSHD_PID" 2>/dev/null || true
  chmod -R u+w "$BASE" 2>/dev/null || true
  rm -rf "$BASE"
  free_after=$(df -B1 --output=avail "$(dirname "$BASE")" | tail -1)
  echo "free space before: $free_before, after: $free_after (difference $(( (free_after - free_before) / 1048576 )) MB)" >&2
}
trap cleanup EXIT

note "generating $N files"
python3 "$HERE/gen-tree.py" flat "$BASE/flat" "$N"
python3 "$HERE/gen-tree.py" tree "$BASE/tree" "$N" --size 1024 --per-dir 1000
python3 "$HERE/gen-tree.py" deep "$BASE/deep" --depth 1000
mkdir -p "$BASE/work"

note "building"
(cd "$ROOT" && "$CARGO" build --release -p rolidscp -q)
TEST_BIN=$(cd "$ROOT" && "$CARGO" test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
  python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')
APP_BIN="$ROOT/target/release/rolidscp"

export ROLIDSCP_BENCH_FLAT="$BASE/flat" ROLIDSCP_BENCH_TREE="$BASE/tree" ROLIDSCP_BENCH_DEEP="$BASE/deep" \
  ROLIDSCP_BENCH_WORK="$BASE/work" ROLIDSCP_BENCH_OUT="$OUT"
run() { note "$1"; RUST_MIN_STACK=16777216 "$TEST_BIN" --ignored --exact "bench::$1" --test-threads=1 --nocapture 2>&1 | grep -v "^BENCH" | grep -E -A2 "panicked|result:|\.result|SKIP" >&2 || true; }
has list && run bench_list_local
has sftp && run bench_list_sftp
has size && run bench_dirsize
has search && run bench_search
has leak && run bench_leak
has deep && run bench_deep
has copy && run bench_copy_delete

if has ref; then
  note "the same work with standard tools"
  S="$BASE/sshd"; mkdir -p "$S"
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
  OPTS=(-i "$S/client" -o UserKnownHostsFile="$S/known_hosts" -o StrictHostKeyChecking=yes -o BatchMode=yes
        -o IdentitiesOnly=yes -o IdentityAgent=none -o UpdateHostKeys=no -F /dev/null)
  SFTP() { sftp -q -b - "${OPTS[@]}" -P "$PORT" "$(id -un)@127.0.0.1"; }
  SSH() { ssh "${OPTS[@]}" -p "$PORT" "$(id -un)@127.0.0.1" "$@"; }

  bench ref.du_sb_ms "$(ms_of du -sb "$BASE/tree")" ms
  bench ref.du_sb.bytes "$(du -sb "$BASE/tree" | cut -f1)" B
  bench ref.find_ms "$(ms_of find "$BASE/tree" -name 'f??00000')" ms
  bench ref.ssh_du_ms "$(ms_of SSH "du -sb '$BASE/tree'")" ms
  bench ref.ssh_find_ms "$(ms_of SSH "find '$BASE/tree' -name 'f??00000'")" ms
  bench ref.sftp_ls_ms "$(ms_of SFTP <<<"ls -1 $BASE/flat")" ms
  mkdir -p "$BASE/ref-up" "$BASE/ref-down"
  bench ref.sftp_put_ms "$(ms_of SFTP <<<"put -r $BASE/tree $BASE/ref-up/")" ms
  bench ref.sftp_get_ms "$(ms_of SFTP <<<"get -r $BASE/ref-up/tree $BASE/ref-down/")" ms
  bench ref.rm_rf_ms "$(ms_of rm -rf "$BASE/ref-down")" ms
  # Delete the uploaded copy over SFTP: the files folder by folder, then the folders.
  { for d in "$BASE/ref-up/tree"/*/; do echo "rm ${d}*"; echo "rmdir $d"; done; echo "rmdir $BASE/ref-up/tree"; } >"$S/rm.batch"
  bench ref.sftp_rm_ms "$(ms_of SFTP <"$S/rm.batch")" ms
  kill "$SSHD_PID"; wait "$SSHD_PID" 2>/dev/null || true; SSHD_PID=
fi

if has gui; then
  if command -v Xvfb >/dev/null; then
    note "frame times in the window"
    "$HERE/gui-frames.sh" "$APP_BIN" "$BASE/flat" "$BASE/gui" | tee -a "$OUT"
  else
    note "Xvfb missing: frame times not measured"
  fi
fi

python3 "$HERE/report.py" "$OUT"
