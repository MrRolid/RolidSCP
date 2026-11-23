#!/usr/bin/env bash
# The AppImage on a minimal system without network, as the AppImage catalog
# tests it: Ubuntu 22.04 in a container with nothing but what the AppImage
# exclude list expects on every system (here: Mesa's EGL and the X11 and
# Wayland client libraries it brings). No libxkbcommon, libXcursor, libXi,
# fonts or keyboard data: the AppImage carries them, the window must appear.
#
# Then the same without a graphics driver: the program must show its start
# error in a window instead of ending silently.
#
# Usage: appimage-minimal.sh <expected version> <AppImage>
# Needs docker, Xvfb and xwininfo on the machine running it; the X server runs
# here and the containers reach it through its socket (they have no network).
set -euo pipefail
EXPECT=${1:?usage: appimage-minimal.sh <version> <AppImage>}
APPIMAGE=$(realpath "${2:?usage: appimage-minimal.sh <version> <AppImage>}")
DISP=:79
WORK=$(mktemp -d)
# -ac: the containers run as another user than this X server.
Xvfb "$DISP" -screen 0 1280x800x24 -nolisten tcp -ac >"$WORK/xvfb.log" 2>&1 &
XVFB_PID=$!
NAMES=()
cleanup() {
    set +e
    for n in "${NAMES[@]}"; do docker rm -f "$n" >/dev/null 2>&1; done
    kill "$XVFB_PID" 2>/dev/null
    wait "$XVFB_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT
for _ in $(seq 50); do DISPLAY=$DISP xwininfo -root >/dev/null 2>&1 && break; sleep 0.2; done

# An image: Ubuntu 22.04 with only these packages (installed with network,
# before the test runs without it).
image() {
    local tag=$1; shift
    local n="rolidscp-prep-$tag"
    NAMES+=("$n")
    docker run --name "$n" ubuntu:22.04 bash -c \
        "apt-get update -q >/dev/null && DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends $* >/dev/null"
    docker commit "$n" "rolidscp-min:$tag" >/dev/null
    docker run --rm "rolidscp-min:$tag" bash -c \
        'echo "libraries: $(ls /usr/lib/x86_64-linux-gnu | grep -cE "xkbcommon|libXi\.|Xcursor|fontconfig") of xkbcommon, Xi, Xcursor, fontconfig; keyboard data: $([ -d /usr/share/X11/xkb ] && echo yes || echo no)"'
}

# Starts the AppImage in the image without network; waits for a window whose
# title contains $2, then checks it is still there after 5 seconds.
run() {
    local tag=$1 want=$2 n="rolidscp-run-$1"
    NAMES+=("$n")
    docker run -d --name "$n" --network none \
        -v /tmp/.X11-unix:/tmp/.X11-unix -v "$APPIMAGE:/app/RolidSCP.AppImage:ro" \
        -e DISPLAY="$DISP" -e APPIMAGE_EXTRACT_AND_RUN=1 -e HOME=/tmp/home -e NO_COLOR=1 \
        "rolidscp-min:$tag" bash -c 'mkdir -p /tmp/home && exec /app/RolidSCP.AppImage' >/dev/null
    local title=""
    for _ in $(seq 60); do
        title=$(LC_ALL=C.UTF-8 DISPLAY=$DISP xwininfo -root -tree 2>/dev/null | grep -o "\"$want[^\"]*\"" | head -1 || true)
        [ -n "$title" ] && break
        [ "$(docker inspect -f '{{.State.Running}}' "$n")" = true ] || break
        sleep 1
    done
    sleep 5
    docker logs "$n" >"$WORK/$tag.log" 2>&1 || true
    sed 's/\x1b\[[0-9;]*m//g' "$WORK/$tag.log"
    if [ -z "$title" ]; then
        echo "FAIL ($tag): no window titled \"$want…\"" >&2
        return 1
    fi
    if [ "$(docker inspect -f '{{.State.Running}}' "$n")" != true ]; then
        echo "FAIL ($tag): the program ended after its window appeared" >&2
        return 1
    fi
    echo "OK ($tag): window $title"
}

echo "== a minimal system: Mesa's EGL only, no network"
image egl libegl1 libegl-mesa0 libgl1-mesa-dri
run egl "RolidSCP $EXPECT"
grep -q "the bundled copy is used" "$WORK/egl.log" || { echo "FAIL: the bundled libraries were not used" >&2; exit 1; }

echo "== no graphics driver: the start error must be shown"
image x11 libx11-6 libx11-xcb1
run x11 "RolidSCP could not start"
grep -q "RolidSCP could not open its window" "$WORK/x11.log" || { echo "FAIL: no start error text on stderr" >&2; exit 1; }
