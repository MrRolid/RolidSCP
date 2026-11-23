#!/bin/bash
# A transfer through the night (roadmap 0.8.19): the program's real window (on
# a private X server, Xvfb) uploads one big sparse file
# (scripts/bench/sparsefile.py) to an existing SSH server under the speed
# limit of its settings, for hours. Once a minute it records the program's
# memory (VmRSS, VmHWM), threads and open files, and how much of the file is on
# the server; once an hour a screenshot. At the end the copy is compared on the
# server by SHA-256 and deleted.
#
#   scripts/bench/overnight.sh [-g GIB] [-l KIB_S] [-h HOURS] BASE REMOTE_DIR OUT_DIR -- PROGRAM
#
# BASE        an empty or missing folder here (the program's home and the
#             sparse source; deleted at the end)
# REMOTE_DIR  a folder on the server that does not exist yet (deleted at the end)
# OUT_DIR     samples.tsv, result.tsv, screenshots, the program's log
# GIB         size of the file (default 100), KIB_S the speed limit in KiB/s
#             (default 4096), HOURS the most it may take (default 12)
# PROGRAM     the program to start (a release build or the AppImage)
#
# The server, from the environment (nothing is written to ~/.ssh; the host
# key must be in BIG_KNOWN_HOSTS already):
#   BIG_HOST, BIG_PORT (default 22), BIG_USER, BIG_KEY, BIG_KNOWN_HOSTS
#
# Needs Xvfb, ImageMagick's import and python3 with libX11/libXtst
# (scripts/bench/xinput.py). Processes are stopped by the PIDs they were
# started with.
set -euo pipefail
GIB=100 LIMIT=4096 HOURS=12
while getopts "g:l:h:" o; do
  case $o in g) GIB=$OPTARG ;; l) LIMIT=$OPTARG ;; h) HOURS=$OPTARG ;; *) exit 2 ;; esac
done
shift $((OPTIND - 1))
[ $# -ge 5 ] && [ "$4" = -- ] || { sed -n '2,26p' "$0"; exit 2; }
: "${BIG_HOST:?}" "${BIG_USER:?}" "${BIG_KEY:?}" "${BIG_KNOWN_HOSTS:?}"
BIG_PORT=${BIG_PORT:-22}
BASE=$(realpath -m "$1"); RDIR=$2; OUT=$(realpath -m "$3"); PROGRAM=$(realpath "$5")
HERE=$(dirname "$(realpath "$0")")
SIZE=$((GIB * 1024 * 1024 * 1024))
case $RDIR in /*) ;; *) echo "REMOTE_DIR must be absolute" >&2; exit 2 ;; esac
if [ -e "$BASE" ] && [ -n "$(ls -A "$BASE")" ]; then echo "$BASE is not empty" >&2; exit 1; fi
mkdir -p "$BASE" "$OUT"
note() { echo "== $(date +%T) $*" >&2; }
result() { echo -e "$1\t$2\t$3" | tee -a "$OUT/result.tsv"; }

OPTS=(-i "$BIG_KEY" -o UserKnownHostsFile="$BIG_KNOWN_HOSTS" -o StrictHostKeyChecking=yes -o BatchMode=yes
      -o IdentitiesOnly=yes -o IdentityAgent=none -o UpdateHostKeys=no -F /dev/null
      -o ControlMaster=auto -o ControlPath="$BASE/cm" -o ControlPersist=300)
R() { ssh "${OPTS[@]}" -p "$BIG_PORT" "$BIG_USER@$BIG_HOST" "$@"; }

XPID= APID=
cleanup() {
  [ -n "$APID" ] && { kill "$APID" 2>/dev/null || true; sleep 2; kill -KILL "$APID" 2>/dev/null || true; wait "$APID" 2>/dev/null || true; }
  [ -n "$XPID" ] && { kill "$XPID" 2>/dev/null || true; wait "$XPID" 2>/dev/null || true; }
  cp "$BASE/log/rolidscp.log" "$OUT/rolidscp.log" 2>/dev/null || true
  R "rm -rf -- '$RDIR'" 2>/dev/null || true
  R -O exit 2>/dev/null || true
  rm -rf "$BASE"
}
trap cleanup EXIT

R "test ! -e '$RDIR' && mkdir -p -- '$RDIR'" || { echo "$RDIR exists on the server or cannot be made" >&2; exit 1; }
free=$(R "df -B1 --output=avail '$RDIR' | tail -1")
[ "$free" -gt $((SIZE / 10 * 11)) ] || { echo "not enough space on the server: $free B" >&2; exit 1; }

note "writing a sparse file of $GIB GiB"
mkdir -p "$BASE/local" "$BASE/home/.ssh" "$BASE/cfg" "$BASE/log" "$BASE/run"
chmod 700 "$BASE/home/.ssh" "$BASE/run"
want=$(python3 -I "$HERE/sparsefile.py" "$BASE/local/big.bin" "$GIB")
cp "$BIG_KNOWN_HOSTS" "$BASE/home/.ssh/known_hosts"
cat >"$BASE/cfg/settings.toml" <<EOF
language = "en"
speed_limit_kib = $LIMIT
EOF
cat >"$BASE/cfg/workspace.toml" <<EOF
active_side = 0

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
host = "$BIG_HOST"
port = $BIG_PORT
user = "$BIG_USER"
key_path = "$BIG_KEY"
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
X() { python3 "$HERE/xinput.py" "$@"; }
shot() { import -window root "$OUT/$1.png" 2>/dev/null || true; }
for _ in $(seq 300); do grep -q "login successful" "$BASE/log/rolidscp.log" 2>/dev/null && break; kill -0 "$APID" || break; sleep 0.2; done
grep -q "login successful" "$BASE/log/rolidscp.log" 2>/dev/null || { shot start; tail -5 "$BASE/app.out" >&2; echo "the program did not log in" >&2; exit 1; }
sleep 2
# The left panel: "..", big.bin. F5 copies it to the server panel.
X focus RolidSCP >/dev/null 2>&1 || true
X key Home; sleep 0.4; X key Down; sleep 0.4; X key F5; sleep 1.2; shot dialog; X key enter
note "transfer started (limit $LIMIT KiB/s)"
result overnight.size "$SIZE" B
result overnight.limit_kib_s "$LIMIT" KiB/s

# One sample a minute until the file is on the server whole.
echo -e "time\tseconds\tremote_bytes\tvmrss_kb\tvmhwm_kb\tthreads\tfds" >"$OUT/samples.tsv"
t0=$(date +%s); hour=0; done_at=
while :; do
  now=$(date +%s)
  kill -0 "$APID" 2>/dev/null || { note "the program ended"; break; }
  got=$(R "stat -c %s -- '$RDIR/big.bin' 2>/dev/null || stat -c %s -- '$RDIR/big.bin.filepart' 2>/dev/null || echo 0" 2>/dev/null || echo -1)
  st=/proc/$APID/status
  echo -e "$(date +%FT%T)\t$((now - t0))\t$got\t$(awk '/^VmRSS/{print $2}' $st)\t$(awk '/^VmHWM/{print $2}' $st)\t$(awk '/^Threads/{print $2}' $st)\t$(ls /proc/$APID/fd | wc -l)" >>"$OUT/samples.tsv"
  if [ $(( (now - t0) / 3600 )) -ge "$hour" ]; then shot "hour-$hour"; hour=$((hour + 1)); fi
  if R "test -f '$RDIR/big.bin'" 2>/dev/null && [ "$(R "stat -c %s -- '$RDIR/big.bin'")" = "$SIZE" ]; then done_at=$now; break; fi
  [ $((now - t0)) -lt $((HOURS * 3600)) ] || { note "not done in $HOURS h"; break; }
  sleep 60
done
shot end
result overnight.finished "$([ -n "$done_at" ] && echo 1 || echo 0)" bool
[ -n "$done_at" ] && result overnight.seconds "$((done_at - t0))" s
result overnight.program_running_at_end "$(kill -0 "$APID" 2>/dev/null && echo 1 || echo 0)" bool
# Memory and open files: the first hour against the last.
python3 -I - "$OUT/samples.tsv" <<'EOF' | while read -r k v u; do result "$k" "$v" "$u"; done
import csv, sys
rows = list(csv.DictReader(open(sys.argv[1]), delimiter="\t"))
if rows:
    first = [r for r in rows if int(r["seconds"]) < 3600] or rows[:1]
    last = [r for r in rows if int(r["seconds"]) >= int(rows[-1]["seconds"]) - 3600]
    for k in ("vmrss_kb", "threads", "fds"):
        a = max(int(r[k]) for r in first); b = max(int(r[k]) for r in last)
        print(f"overnight.{k}.first_hour_max {a} {k.split('_')[-1]}")
        print(f"overnight.{k}.last_hour_max {b} {k.split('_')[-1]}")
        print(f"overnight.{k}.max {max(int(r[k]) for r in rows)} {k.split('_')[-1]}")
    print(f"overnight.vmhwm_kb.end {rows[-1]['vmhwm_kb']} kb")
    print(f"overnight.samples {len(rows)} n")
EOF
if [ -n "$done_at" ]; then
  note "SHA-256 on the server"
  got=$(R "sha256sum -- '$RDIR/big.bin'" | cut -d' ' -f1)
  result overnight.sha256_equal "$([ "$got" = "$want" ] && echo 1 || echo 0)" bool
fi
result overnight.log_errors "$(grep -c ' ERROR ' "$BASE/log/rolidscp.log" || true)" n
result overnight.reconnections "$(grep -cE 'connection .* restored' "$BASE/log/rolidscp.log" || true)" n
note "done"
