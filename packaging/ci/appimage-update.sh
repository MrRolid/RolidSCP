#!/usr/bin/env bash
# The AppImage's update information and .zsync file, checked with
# AppImageUpdate's appimageupdatetool (a fixed release, checked against its
# SHA-256):
#   1. the AppImage carries the update information for GitHub releases of
#      MrRolid/RolidSCP, and its pattern matches the .zsync file's name;
#   2. an older AppImage pointed at the .zsync file (served here, with range
#      requests as GitHub serves it) is told an update is there, and updating
#      it gives a file equal to the new AppImage.
#
# Usage: appimage-update.sh <AppImage> (its .zsync file next to it)
set -euo pipefail
APPIMAGE=$(realpath "${1:?usage: appimage-update.sh <AppImage>}")
NAME=$(basename "$APPIMAGE")
[ -f "$APPIMAGE.zsync" ] || { echo "FAIL: no $NAME.zsync" >&2; exit 1; }
TOOL_URL=https://github.com/AppImageCommunity/AppImageUpdate/releases/download/2.0.0-alpha-1-20251018/appimageupdatetool-x86_64.AppImage
TOOL_SHA256=d976cdac667b03dee8cb23fb95ef74b042c406c5cbab3ff294d2b16efeaff84f
WORK=$(mktemp -d)
HTTP_PID=""
cleanup() { set +e; [ -n "$HTTP_PID" ] && kill "$HTTP_PID" && wait "$HTTP_PID"; rm -rf "$WORK"; }
trap cleanup EXIT
export APPIMAGE_EXTRACT_AND_RUN=1
TOOL=$WORK/appimageupdatetool
curl -fsSL --retry 3 -o "$TOOL" "$TOOL_URL"
echo "$TOOL_SHA256  $TOOL" | sha256sum -c --quiet
chmod +x "$TOOL"

echo "== the update information"
"$TOOL" -d "$APPIMAGE" 2>&1 | grep -E "^(Raw update information|Update information type)" | tee "$WORK/describe"
INFO=$(sed -n 's/^Raw update information: //p' "$WORK/describe")
IFS='|' read -r kind user repo tag pattern <<<"$INFO"
[ "$kind|$user|$repo|$tag" = "gh-releases-zsync|MrRolid|RolidSCP|latest-all" ] \
    || { echo "FAIL: unexpected update information: $INFO" >&2; exit 1; }
# shellcheck disable=SC2053 # the pattern is meant to match as a glob
[[ "$NAME.zsync" == $pattern ]] || { echo "FAIL: $pattern does not match $NAME.zsync" >&2; exit 1; }
echo "OK: $pattern matches $NAME.zsync"

echo "== an update from an older AppImage"
mkdir "$WORK/srv" "$WORK/old"
cp "$APPIMAGE" "$APPIMAGE.zsync" "$WORK/srv/"
PORT=18765
python3 -I "$(dirname "$0")/rangeserver.py" "$WORK/srv" "$PORT" 2>"$WORK/http.log" &
HTTP_PID=$!
# The older AppImage: this one with other update information (so its content
# differs), pointing at the served .zsync file.
OLD=$WORK/old/RolidSCP-old.AppImage
cp "$APPIMAGE" "$OLD"
read -r off size < <(readelf -S -W "$OLD" | sed -n 's/.*\] \.upd_info  *[A-Z]*  *[0-9a-f]*  *\([0-9a-f]*\)  *\([0-9a-f]*\) .*/\1 \2/p')
dd if=/dev/zero of="$OLD" bs=1 seek="$((16#$off))" count="$((16#$size))" conv=notrunc status=none
printf 'zsync|http://127.0.0.1:%s/%s.zsync' "$PORT" "$NAME" | dd of="$OLD" bs=1 seek="$((16#$off))" conv=notrunc status=none
for _ in $(seq 50); do curl -fsI "http://127.0.0.1:$PORT/$NAME.zsync" >/dev/null 2>&1 && break; sleep 0.1; done
set +e
"$TOOL" -j "$OLD" >"$WORK/check.log" 2>&1
rc=$?
set -e
[ "$rc" = 1 ] || { cat "$WORK/check.log"; echo "FAIL: no update found (exit $rc)" >&2; exit 1; }
echo "OK: an update was found"
(cd "$WORK/old" && "$TOOL" "$(basename "$OLD")" 2>&1 | tr '\r' '\n' | grep -v '% done' | tail -3)
[ "$(sha256sum <"$WORK/old/$NAME")" = "$(sha256sum <"$APPIMAGE")" ] \
    || { echo "FAIL: the updated file differs from $NAME" >&2; exit 1; }
echo "OK: the updated file equals $NAME"
