#!/bin/bash
# A transfer through a real outage made by hand (roadmap 0.8.19): the network
# cable of the server pulled out, or the server put to sleep, in the middle of
# a transfer. The program's real window (on a private X server, Xvfb) copies
# one sparse file (scripts/bench/sparsefile.py) to or from an existing SSH
# server under a speed limit. When a fifth of the file has gone the script
# prints "ACTION NOW" (and writes OUT_DIR/action) and goes on watching: once a
# second how much of the file has arrived, and what the program logs about
# its connection. When the copy is whole it is compared by SHA-256.
#
#   scripts/bench/outage.sh [-d up|down] [-g GIB] [-l KIB_S] [-m MINUTES] BASE REMOTE_DIR OUT_DIR -- PROGRAM
#
# BASE        an empty or missing folder here (the program's home, the source
#             or the copy; deleted at the end)
# REMOTE_DIR  a folder on the server that does not exist yet (deleted at the end)
# OUT_DIR     progress.tsv (one line a second), result.tsv, screenshots, the
#             program's log
# -d          up (here → server, default) or down (server → here)
# GIB         size of the file (default 6), KIB_S the speed limit (default
#             20480), MINUTES the most it may take (default 40)
# PROGRAM     the program to start
#
# The server, from the environment (nothing is written to ~/.ssh; the host
# key must be in BIG_KNOWN_HOSTS already):
#   BIG_HOST, BIG_PORT (default 22), BIG_USER, BIG_KEY, BIG_KNOWN_HOSTS
# For -d down the server needs python3 and this folder's sparsefile.py, at
# BIG_SPARSEFILE (default ~/rolid/scripts/bench/sparsefile.py there).
#
# Through a jump host (0.9.1): BIG_JUMP_HOST, BIG_JUMP_PORT (default 22),
# BIG_JUMP_USER, BIG_JUMP_KEY. The program then connects to BIG_TARGET_HOST
# (default 127.0.0.1: the jump host itself), BIG_PORT, as the jump host sees
# it; this script still watches the files at BIG_HOST directly, so BIG_HOST
# must be the machine the target is. BIG_KNOWN_HOSTS needs the keys of both
# names.
#
# Needs Xvfb, ImageMagick's import and python3 with libX11/libXtst
# (scripts/bench/xinput.py). Processes are stopped by the PIDs they were
# started with.
set -euo pipefail
WAY=up GIB=6 LIMIT=20480 MINUTES=40
while getopts "d:g:l:m:" o; do
  case $o in d) WAY=$OPTARG ;; g) GIB=$OPTARG ;; l) LIMIT=$OPTARG ;; m) MINUTES=$OPTARG ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 5 ] && [ "$4" = -- ] || { sed -n '2,31p' "$0"; exit 2; }
case $WAY in up | down) ;; *) echo "-d up or -d down" >&2; exit 2 ;; esac
: "${BIG_HOST:?}" "${BIG_USER:?}" "${BIG_KEY:?}" "${BIG_KNOWN_HOSTS:?}"
BIG_PORT=${BIG_PORT:-22}
BASE=$(realpath -m "$1"); RDIR=$2; OUT=$(realpath -m "$3"); PROGRAM=$(realpath "$5")
HERE=$(dirname "$(realpath "$0")")
SIZE=$((GIB * 1024 * 1024 * 1024))
case $RDIR in /*) ;; *) echo "REMOTE_DIR must be absolute" >&2; exit 2 ;; esac
if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$BASE" "$OUT"
rm -f "$OUT/action"
note() { echo "== $(date +%T) $*" >&2; }
result() { echo -e "$1\t$2\t$3" | tee -a "$OUT/result.tsv"; }

# Short tries only: while the line is down a poll must not hang.
OPTS=(-i "$BIG_KEY" -o UserKnownHostsFile="$BIG_KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes
      -o IdentitiesOnly=yes -o IdentityAgent=none -o UpdateHostKeys=no -F /dev/null -o ConnectTimeout=3)
R() { timeout 8 ssh "${OPTS[@]}" -p "$BIG_PORT" "$BIG_USER@$BIG_HOST" "$@"; }
RL() { ssh "${OPTS[@]}" -o ConnectTimeout=20 -p "$BIG_PORT" "$BIG_USER@$BIG_HOST" "$@"; }

XPID= APID=
cleanup() {
  [ -n "$APID" ] && { kill "$APID" 2>/dev/null || true; sleep 2; kill -KILL "$APID" 2>/dev/null || true; wait "$APID" 2>/dev/null || true; }
  [ -n "$XPID" ] && { kill "$XPID" 2>/dev/null || true; wait "$XPID" 2>/dev/null || true; }
  cp "$BASE/log/rolidscp.log" "$OUT/rolidscp.log" 2>/dev/null || true
  RL "rm -rf -- '$RDIR'" 2>/dev/null || true
  rm -rf "$BASE"
}
trap cleanup EXIT

RL "test ! -e '$RDIR' && mkdir -p -- '$RDIR'" || { echo "$RDIR exists on the server or cannot be made" >&2; exit 1; }
mkdir -p "$BASE/local" "$BASE/home/.ssh" "$BASE/cfg" "$BASE/log" "$BASE/run"
chmod 700 "$BASE/home/.ssh" "$BASE/run"
if [ "$WAY" = up ]; then
  note "writing a sparse file of $GIB GiB here"
  want=$(python3 -I "$HERE/sparsefile.py" "$BASE/local/big.bin" "$GIB")
  # Where the copy grows: on the server.
  size_now() { R "stat -c %s -- '$RDIR/big.bin' 2>/dev/null || stat -c %s -- '$RDIR/big.bin.filepart' 2>/dev/null || echo 0" 2>/dev/null || echo -1; }
  whole() { RL "test -f '$RDIR/big.bin' && stat -c %s -- '$RDIR/big.bin'" 2>/dev/null | grep -qx "$SIZE"; }
  copy_sum() { RL "sha256sum -- '$RDIR/big.bin'" | cut -d' ' -f1; }
else
  note "writing a sparse file of $GIB GiB on the server"
  want=$(RL "python3 -I ${BIG_SPARSEFILE:-~/rolid/scripts/bench/sparsefile.py} '$RDIR/big.bin' '$GIB'")
  size_now() { stat -c %s "$BASE/local/big.bin" 2>/dev/null || stat -c %s "$BASE/local/big.bin.filepart" 2>/dev/null || echo 0; }
  whole() { [ -f "$BASE/local/big.bin" ] && [ "$(stat -c %s "$BASE/local/big.bin")" = "$SIZE" ]; }
  copy_sum() { sha256sum "$BASE/local/big.bin" | cut -d' ' -f1; }
fi
cp "$BIG_KNOWN_HOSTS" "$BASE/home/.ssh/known_hosts"
cat >"$BASE/cfg/settings.toml" <<EOF
language = "en"
speed_limit_kib = $LIMIT
EOF
TARGET_HOST=$BIG_HOST JUMP_TOML=
if [ -n "${BIG_JUMP_HOST:-}" ]; then
  : "${BIG_JUMP_USER:?}" "${BIG_JUMP_KEY:?}"
  TARGET_HOST=${BIG_TARGET_HOST:-127.0.0.1}
  JUMP_TOML="
[right.tab.location.target.jump]
host = \"$BIG_JUMP_HOST\"
port = ${BIG_JUMP_PORT:-22}
user = \"$BIG_JUMP_USER\"
key_path = \"$BIG_JUMP_KEY\""
fi
cat >"$BASE/cfg/workspace.toml" <<EOF
active_side = $([ "$WAY" = up ] && echo 0 || echo 1)

[left]
active = 0

[[left.tab]]
color = "#FFFFFF"

[left.tab.location]
kind = "local"
path = "$BASE/local"

[right]
active = 0

[[right.tab]]
color = "#3070C0"

[right.tab.location]
kind = "remote"
path = "$RDIR"

[right.tab.location.target]
host = "$TARGET_HOST"
port = $BIG_PORT
user = "$BIG_USER"
key_path = "$BIG_KEY"
$JUMP_TOML
EOF

DISP=
for n in $(seq 140 199); do
  if [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then DISP=$n; break; fi
done
[ -n "$DISP" ] || { echo "no free X display" >&2; exit 1; }
Xvfb ":$DISP" -screen 0 1400x900x24 -nolisten tcp >"$BASE/xvfb.log" 2>&1 &
XPID=$!
for _ in $(seq 50); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.1; done
export DISPLAY=":$DISP"
env -i PATH=/usr/bin:/bin HOME="$BASE/home" XDG_RUNTIME_DIR="$BASE/run" ROLIDSCP_CONFIG_DIR="$BASE/cfg" ROLIDSCP_LOG_DIR="$BASE/log" \
  DISPLAY=":$DISP" LANG=C.UTF-8 "$PROGRAM" >"$BASE/app.out" 2>&1 &
APID=$!
LOG=$BASE/log/rolidscp.log
X() { python3 "$HERE/xinput.py" "$@"; }
shot() { import -window root "$OUT/$1.png" 2>/dev/null || true; }
# Through a jump host: two logins (the jump host, then the server).
LOGINS=$([ -n "${BIG_JUMP_HOST:-}" ] && echo 2 || echo 1)
logged_in() { local n; n=$(grep -c "login successful" "$LOG" 2>/dev/null) || true; [ "${n:-0}" -ge "$LOGINS" ]; }
for _ in $(seq 300); do logged_in && break; kill -0 "$APID" || break; sleep 0.2; done
logged_in || { shot start; tail -5 "$BASE/app.out" >&2; echo "the program did not log in" >&2; exit 1; }
sleep 2
# The active panel (local for up, the server for down): "..", big.bin; F5.
X focus RolidSCP >/dev/null 2>&1 || true
X key Home; sleep 0.4; X key Down; sleep 0.4; X key F5; sleep 1.2; shot dialog; X key enter
note "transfer $WAY started ($GIB GiB, limit $LIMIT KiB/s)"
result outage.way "$WAY" -
result outage.jump_host "$([ -n "${BIG_JUMP_HOST:-}" ] && echo 1 || echo 0)" bool
result outage.size "$SIZE" B

echo -e "time\tseconds\tbytes\tlog_lost\tlog_restored" >"$OUT/progress.tsv"
t0=$(date +%s); acted=; done_at=; shots=0
while :; do
  now=$(date +%s)
  kill -0 "$APID" 2>/dev/null || { note "the program ended"; break; }
  got=$(size_now)
  lost=$(grep -c "connection .* lost" "$LOG" || true); back=$(grep -cE "connection .* restored" "$LOG" || true)
  echo -e "$(date +%T)\t$((now - t0))\t$got\t$lost\t$back" >>"$OUT/progress.tsv"
  if [ -z "$acted" ] && [ "$got" -ge $((SIZE / 5)) ]; then
    acted=$now; date +%T >"$OUT/action"
    note "ACTION NOW (a fifth of the file is there)"
    shot action
  fi
  # Screenshots while the program waits for the line (once a minute, up to 10).
  if [ -n "$acted" ] && [ "$lost" -gt "$back" ] && [ $(( (now - acted) % 60 )) -lt 2 ] && [ "$shots" -lt 10 ]; then
    shots=$((shots + 1)); shot "waiting-$shots"
  fi
  if [ "$got" = "$SIZE" ] && whole; then done_at=$now; break; fi
  [ $((now - t0)) -lt $((MINUTES * 60)) ] || { note "not done in $MINUTES min"; break; }
  sleep 1
done
shot end
result outage.finished "$([ -n "$done_at" ] && echo 1 || echo 0)" bool
[ -n "$done_at" ] && result outage.seconds "$((done_at - t0))" s
result outage.program_running_at_end "$(kill -0 "$APID" 2>/dev/null && echo 1 || echo 0)" bool
result outage.log_lost "$(grep -c "connection .* lost" "$LOG" || true)" n
result outage.log_restored "$(grep -cE "connection .* restored" "$LOG" || true)" n
result outage.log_errors "$(grep -c ' ERROR ' "$LOG" || true)" n
# The outage as the progress saw it: the longest time without growth after
# the action, and how long after it the copy grew again.
python3 -I - "$OUT/progress.tsv" <<'EOF' | while read -r k v u; do result "$k" "$v" "$u"; done
import csv, sys
rows = [r for r in csv.DictReader(open(sys.argv[1]), delimiter="\t")]
best = (0, None, None)
last_t, last_b = None, None
for r in rows:
    t, b = int(r["seconds"]), int(r["bytes"])
    if b < 0:
        continue
    if last_b is not None and b > last_b:
        if t - last_t > best[0]:
            best = (t - last_t, last_t, t)
        last_t, last_b = t, b
    elif last_b is None:
        last_t, last_b = t, b
print(f"outage.longest_stall {best[0]} s")
if best[1] is not None:
    print(f"outage.stall_from {best[1]} s")
    print(f"outage.stall_to {best[2]} s")
EOF
if [ -n "$done_at" ]; then
  note "SHA-256"
  got=$(copy_sum)
  result outage.sha256_equal "$([ "$got" = "$want" ] && echo 1 || echo 0)" bool
fi
grep -E "connection .* (lost|restored)|reconnecting|WARN rolidscp::app: (copy|transfer)" "$LOG" | cut -c1-220 >"$OUT/connection.log" || true
note "done"
