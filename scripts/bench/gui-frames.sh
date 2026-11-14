#!/bin/bash
# Frame times of the real window with a big folder, on a private X server (Xvfb).
#
#   gui-frames.sh BINARY FOLDER WORKDIR
#
# Starts Xvfb and the program with a throwaway HOME and configuration in WORKDIR
# (never the user's own), opens FOLDER in the left panel and waits until it is
# listed. Then scrolls with PgDn, the mouse wheel, End/Home and types into the
# quick filter, and prints BENCH lines (as the Rust load tests do):
#   gui.list_shown_ms   when the folder was shown (from the program's log)
#   gui.<step>.avg_ms / gui.<step>.max_ms   frame times during each step
#   gui.rss_mb / gui.hwm_mb                  memory of the program
# Needs Xvfb, python3, libX11 and libXtst. Processes are stopped by their PIDs.
set -euo pipefail
BIN=$(realpath "$1"); FOLDER=$(realpath "$2"); W=$(realpath -m "$3")
HERE=$(dirname "$(realpath "$0")")
mkdir -p "$W/home" "$W/cfg" "$W/log" "$W/run"
chmod 700 "$W/run"
cat > "$W/cfg/workspace.toml" <<EOF
active_side = 0

[left]
active = 0

[[left.tab]]
color = "#FFFFFF"

[left.tab.location]
kind = "local"
path = "$FOLDER"

[right]
active = 0

[[right.tab]]
color = "#FFFFFF"

[right.tab.location]
kind = "local"
path = "$W/home"
EOF

# A free display number.
DISP=
for n in $(seq 140 199); do
  if [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then DISP=$n; break; fi
done
[ -n "$DISP" ] || { echo "no free X display" >&2; exit 1; }
Xvfb ":$DISP" -screen 0 1600x1000x24 -nolisten tcp >"$W/xvfb.log" 2>&1 &
XPID=$!
APID=
cleanup() {
  [ -n "$APID" ] && kill "$APID" 2>/dev/null || true
  [ -n "$APID" ] && wait "$APID" 2>/dev/null || true
  kill "$XPID" 2>/dev/null || true
  wait "$XPID" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 50); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.1; done

env -i PATH=/usr/bin:/bin HOME="$W/home" XDG_RUNTIME_DIR="$W/run" ROLIDSCP_CONFIG_DIR="$W/cfg" \
  ROLIDSCP_LOG_DIR="$W/log" ROLIDSCP_FRAME_STATS=1 DISPLAY=":$DISP" LANG=en_US.UTF-8 \
  "$BIN" >"$W/app.out" 2>&1 &
APID=$!

# Wait for the listing: the program's log line, or memory that stopped growing.
LOG="$W/log/rolidscp.log"
start=$(date +%s%N)
last=0; still=0
for _ in $(seq 1200); do
  sleep 0.5
  kill -0 "$APID" 2>/dev/null || { echo "the program ended:" >&2; tail -20 "$W/app.out" >&2; exit 1; }
  if grep -q "folder listed:" "$LOG" 2>/dev/null; then break; fi
  rss=$(awk '/VmRSS/ {print $2}' "/proc/$APID/status")
  if [ "$rss" -gt 200000 ] && [ $(( rss - last )) -lt 2000 ]; then still=$((still + 1)); else still=0; fi
  last=$rss
  [ "$still" -ge 10 ] && break
done
waited=$(( ($(date +%s%N) - start) / 1000000 ))
shown=$(grep -o "folder listed: .*shown after [0-9]* ms" "$LOG" 2>/dev/null | grep -o "[0-9]* ms$" | tr -dc 0-9 || true)
echo -e "BENCH\tgui.list_shown_ms\t${shown:-$waited}\tms"
sleep 3

export DISPLAY=":$DISP"
X() { python3 "$HERE/xinput.py" "$@"; }
X focus RolidSCP move 400 500
# Frame statistics are logged every 2 s while frames are drawn: each step runs
# for a while and its lines are taken from the log afterwards.
step() {
  local name=$1; shift
  local from; from=$(wc -l <"$LOG")
  "$@"
  sleep 2.5
  # No frames (nothing changed on screen) is a result too, not an error.
  { tail -n +"$((from + 1))" "$LOG" | grep -o "frames: [0-9]* in 2 s, average [0-9.]* ms, longest [0-9.]* ms" || true; } |
    awk -v n="$name" '{f+=$2; t+=$2*$7; if ($10>m) m=$10} END {
      if (f) { printf "BENCH\tgui.%s.avg_ms\t%.3f\tms\nBENCH\tgui.%s.max_ms\t%.3f\tms\nBENCH\tgui.%s.frames\t%d\tframes\n", n, t/f, n, m, n, f }
      else   { printf "BENCH\tgui.%s.frames\t0\tframes\n", n } }'
}
pgdn() { for _ in $(seq 60); do X key pgdn; done; }
wheel() { X wheel down 150; }
ends() { for _ in $(seq 5); do X key end sleep 0.3 key home sleep 0.3; done; }
filter() { X type f12 sleep 0.5 key bs key bs key bs sleep 0.5 type f9 sleep 0.5 key esc; }
mark() { X key ctrl+a sleep 1 key pgdn key pgdn key pgdn sleep 1 key ctrl+a; }
step pgdn pgdn
step wheel wheel
step ends ends
step filter filter
step marked mark
step idle sleep 3
echo -e "BENCH\tgui.rss_mb\t$(awk '/VmRSS/ {printf "%.1f", $2/1024}' "/proc/$APID/status")\tMB"
echo -e "BENCH\tgui.hwm_mb\t$(awk '/VmHWM/ {printf "%.1f", $2/1024}' "/proc/$APID/status")\tMB"
{ grep -c "slow frame" "$LOG" || true; } | awk '{printf "BENCH\tgui.slow_frames\t%d\tframes\n", $1}'
