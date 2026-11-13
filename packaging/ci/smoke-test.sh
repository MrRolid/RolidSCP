#!/usr/bin/env bash
# Start RolidSCP on a virtual X server with an empty home folder and check that
# it starts, stays running and reports the expected version (start-up log line
# and window title).
#
# Usage: smoke-test.sh <expected version> <command> [args...]
set -euo pipefail
EXPECT=${1:?usage: smoke-test.sh <version> <command> [args...]}
shift

WORK=$(mktemp -d)
mkdir -m 700 "$WORK/home" "$WORK/run"
export HOME=$WORK/home XDG_RUNTIME_DIR=$WORK/run
unset WAYLAND_DISPLAY
export NO_COLOR=1
export DISPLAY=:77

Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >"$WORK/xvfb.log" 2>&1 &
XVFB_PID=$!
APP_PID=
cleanup() {
    set +e
    if [ -n "$APP_PID" ]; then kill "$APP_PID" 2>/dev/null; wait "$APP_PID" 2>/dev/null; fi
    kill "$XVFB_PID" 2>/dev/null
    wait "$XVFB_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT
for _ in $(seq 1 50); do
    xwininfo -root >/dev/null 2>&1 && break
    sleep 0.2
done
if ! kill -0 "$XVFB_PID" 2>/dev/null || ! xwininfo -root >/dev/null 2>&1; then
    echo "FAIL: Xvfb did not start on $DISPLAY" >&2
    cat "$WORK/xvfb.log" >&2
    exit 1
fi

"$@" >"$WORK/app.log" 2>&1 &
APP_PID=$!

TITLE=""
for _ in $(seq 1 60); do
    if ! kill -0 "$APP_PID" 2>/dev/null; then
        echo "FAIL: the program ended during start-up" >&2
        cat "$WORK/app.log" >&2
        exit 1
    fi
    TITLE=$(LC_ALL=C.UTF-8 xwininfo -root -tree 2>/dev/null | grep -o "\"RolidSCP $EXPECT[^\"]*\"" | head -1 || true)
    [ -n "$TITLE" ] && break
    sleep 1
done
# A few more seconds: the first frames must not crash it.
sleep 5
if ! kill -0 "$APP_PID" 2>/dev/null; then
    echo "FAIL: the program ended after start-up" >&2
    cat "$WORK/app.log" >&2
    exit 1
fi
# Without colour codes, in case the log has them anyway.
sed 's/\x1b\[[0-9;]*m//g' "$WORK/app.log" > "$WORK/app.txt"
cat "$WORK/app.txt"
if ! grep -q "RolidSCP start.*version=\"\?$EXPECT\"\?" "$WORK/app.txt"; then
    echo "FAIL: no start-up log line with version $EXPECT" >&2
    exit 1
fi
if [ -z "$TITLE" ]; then
    echo "FAIL: no window titled \"RolidSCP $EXPECT ...\"" >&2
    LC_ALL=C.UTF-8 xwininfo -root -tree >&2 || true
    exit 1
fi
echo "OK: RolidSCP $EXPECT started, window title $TITLE"
