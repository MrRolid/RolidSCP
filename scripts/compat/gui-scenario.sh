#!/bin/bash
# The real window of an installed program against a throwaway SSH server:
# connect, list, upload, download (compared by SHA-256), pack to the server
# and delete there, all with the keyboard as a user would, on a private X
# server (Xvfb). Used by distros.sh for every distribution; works on this
# computer too.
#
#   scripts/compat/gui-scenario.sh WORK SHOTS -- LAUNCH...
#
# WORK    a new folder; it must be seen at the same path by the program
#         (distros.sh binds it into the distribution)
# SHOTS   where the screenshots go
# LAUNCH  how to start the program; it gets a clean environment (`env -i`:
#         no ssh-agent, no settings of this session) with HOME,
#         ROLIDSCP_CONFIG_DIR, ROLIDSCP_LOG_DIR, DISPLAY and PATH=$APP_PATH
#         (default /usr/bin:/bin)
#
# Prints `GUI <step> <ok|fail> <detail>` lines; exits 1 when a step failed.
# Needs Xvfb, ImageMagick's import, python3 with libX11/libXtst and sshd.
# Every process is stopped by the PID it was started with.
set -euo pipefail
[ $# -ge 3 ] && [ "$3" = -- ] || { sed -n '2,19p' "$0"; exit 2; }
W=$(realpath -m "$1"); SHOTS=$(realpath -m "$2"); shift 3
HERE=$(dirname "$(realpath "$0")")
X="python3 $HERE/../bench/xinput.py"
mkdir -p "$W" "$SHOTS"
[ -z "$(ls -A "$W")" ] || { echo "$W is not empty" >&2; exit 1; }
FAILED=0
say() { echo -e "GUI\t$1\t$2\t$3"; [ "$2" = ok ] || FAILED=1; }

# The server (this computer's OpenSSH as the normal user).
. "$HERE/../bench/sshd-lib.sh"
start_sshd "$W/sshd" >/dev/null
IFS='|' read -r PORT SUSER SKEY SKH <<<"$ROLIDSCP_BIG_SSH"

# The program's home: its key and the server's key (a throwaway HOME).
mkdir -p "$W/home/.ssh" "$W/cfg" "$W/log" "$W/run" "$W/local/up/sub" "$W/remote/down"
chmod 700 "$W/home/.ssh" "$W/run"
cp "$SKEY" "$W/home/.ssh/id_ed25519"; cp "$SKEY.pub" "$W/home/.ssh/id_ed25519.pub"
cp "$SKH" "$W/home/.ssh/known_hosts"
gen() { head -c "$2" /dev/urandom >"$1"; }
gen "$W/local/up/big.bin" $((5 << 20))
for i in 1 2 3 4 5; do gen "$W/local/up/sub/f$i.txt" $((i * 1000)); done
gen "$W/remote/down/data.bin" $((3 << 20))
for i in 1 2 3; do gen "$W/remote/down/r$i.txt" $((i * 777)); done
cat >"$W/cfg/workspace.toml" <<EOF
active_side = 0

[left]
active = 0

[[left.tab]]
color = "#FFFFFF"

[left.tab.location]
kind = "local"
path = "$W/local"

[right]
active = 0

[[right.tab]]
color = "#3070C0"

[right.tab.location]
kind = "remote"
path = "$W/remote"

[right.tab.location.target]
host = "127.0.0.1"
port = $PORT
user = "$SUSER"
EOF

DISP=
for n in $(seq 140 199); do
  if [ ! -e "/tmp/.X11-unix/X$n" ] && [ ! -e "/tmp/.X$n-lock" ]; then DISP=$n; break; fi
done
[ -n "$DISP" ] || { echo "no free X display" >&2; exit 1; }
Xvfb ":$DISP" -screen 0 1400x900x24 -nolisten tcp >"$W/xvfb.log" 2>&1 &
XPID=$!
APID=
cleanup() {
  [ -n "$APID" ] && { kill "$APID" 2>/dev/null || true; sleep 1; kill -KILL "$APID" 2>/dev/null || true; wait "$APID" 2>/dev/null || true; }
  kill "$XPID" 2>/dev/null || true; wait "$XPID" 2>/dev/null || true
  kill "$SSHD_PID" 2>/dev/null || true; wait "$SSHD_PID" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 50); do [ -e "/tmp/.X11-unix/X$DISP" ] && break; sleep 0.1; done
export DISPLAY=":$DISP"

"$@" env -i PATH="${APP_PATH:-/usr/bin:/bin}" HOME="$W/home" XDG_RUNTIME_DIR="$W/run" ROLIDSCP_CONFIG_DIR="$W/cfg" ROLIDSCP_LOG_DIR="$W/log" \
  DISPLAY=":$DISP" LANG=C.UTF-8 rolidscp >"$W/app.out" 2>&1 &
APID=$!
LOG="$W/log/rolidscp.log"
shot() { import -window root "$SHOTS/$1.png" 2>/dev/null || true; }
# Wait until `cond` holds (a shell test), up to $1 seconds.
until_ok() {
  local t=$1; shift
  for _ in $(seq $((t * 5))); do
    eval "$@" && return 0
    kill -0 "$APID" 2>/dev/null || return 1
    sleep 0.2
  done
  return 1
}
if until_ok 60 'grep -q "login successful" "$LOG" 2>/dev/null'; then
  sleep 2
  say start ok "window open, logged in to the server"
else
  shot start; say start fail "$(tail -3 "$W/app.out" | tr '\n' ' ')"; exit 1
fi
sleep 1
shot start

X() { $X "$@"; }
key() { for k in "$@"; do X key "$k"; sleep 0.4; done; }
# Same files with the same SHA-256 under two folders.
same_tree() {
  [ -d "$1" ] && [ -d "$2" ] || return 1
  [ "$(cd "$1" && find . -type f -exec sha256sum {} + | sort -k2)" = "$(cd "$2" && find . -type f -exec sha256sum {} + | sort -k2)" ]
}
# A step: name, keys…, then the condition to wait for (after --).
step() {
  local name=$1; shift
  local keys=()
  while [ "$1" != -- ]; do keys+=("$1"); shift; done
  shift
  X focus RolidSCP >/dev/null 2>&1 || true
  key "${keys[@]}"
  sleep 0.6; shot "$name-dialog"
  key enter
  if until_ok 90 "$@"; then sleep 0.5; shot "$name"; say "$name" ok "${DETAIL:-done}"
  else shot "$name"; say "$name" fail "${DETAIL:-not done in 90 s}"; fi
  DETAIL=
}
X focus RolidSCP >/dev/null 2>&1 || true

# Download: the right (server) panel, folder "down", F5.
DETAIL="3 MiB and 3 files, SHA-256 equal"
step download tab home down F5 -- 'same_tree "$W/remote/down" "$W/local/down"'
# Upload: the left panel, folder "up" (.., down, up), F5.
DETAIL="5 MiB and a subfolder, SHA-256 equal"
step upload tab home down down F5 -- 'same_tree "$W/local/up" "$W/remote/up"'
# A new folder on the server: right panel, F7.
X focus RolidSCP >/dev/null 2>&1 || true
key tab F7; sleep 0.6; X type made; sleep 0.3; shot newfolder-dialog; key enter
if until_ok 30 '[ -d "$W/remote/made" ]'; then say newfolder ok "made on the server"; else say newfolder fail "no folder"; fi
shot newfolder
# Pack to the server: the archive goes to the other panel; left (.., down, up), "up", Alt+F5.
DETAIL="up.zip written on the server"
step pack tab home down down alt+F5 -- '[ -s "$W/remote/up.zip" ] && python3 -I -m zipfile -t "$W/remote/up.zip" >/dev/null 2>&1'
mkdir -p "$W/unzipped"
if python3 -I -m zipfile -e "$W/remote/up.zip" "$W/unzipped" 2>/dev/null && same_tree "$W/local/up" "$W/unzipped/up"; then
  say archive ok "the archive holds the folder, SHA-256 equal"
else say archive fail "content differs ($(ls "$W/unzipped" 2>/dev/null | tr '\n' ' '))"; fi
# Delete on the server: right panel (.., down, made, up, up.zip), "made", F8.
DETAIL="folder gone"
step delete tab home down down F8 -- '[ ! -e "$W/remote/made" ]'

kill -0 "$APID" 2>/dev/null && say running ok "the program is still running" || say running fail "the program ended"
grep -E " (ERROR|WARN) rolid" "$LOG" | grep -v "slow frame" | cut -c1-200 | sed 's/^/LOG\t/' || true
exit $FAILED
