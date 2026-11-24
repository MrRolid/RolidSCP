#!/bin/bash
# Live check of the AppImage update: an older release is updated by
# AppImageUpdate to the newest one on GitHub, and the result is checked
# against the newest release's SHA256SUMS (and its signature, when the
# release has SHA256SUMS.asc).
#
#   scripts/release/appimage-update-check.sh <old version> <expected new version> <empty folder>
#   e.g.  appimage-update-check.sh 0.8.19 0.8.20 ~/aiu-test
#
# Changes nothing outside the folder; needs curl, sha256sum and (for the
# signature) gpg. The tool runs with --appimage-extract-and-run, so FUSE is
# not needed. Run it after the new release is published.
set -euo pipefail
OLD=${1:?old version}
NEW=${2:?expected new version}
DIR=${3:?empty folder}
REPO=MrRolid/RolidSCP
TOOL_URL=https://github.com/AppImageCommunity/AppImageUpdate/releases/download/2.0.0-alpha-1-20251018/appimageupdatetool-x86_64.AppImage
TOOL_SHA256=d976cdac667b03dee8cb23fb95ef74b042c406c5cbab3ff294d2b16efeaff84f
KEY_URL=https://raw.githubusercontent.com/$REPO/main/docs/release-key.asc

mkdir -p "$DIR"
cd "$DIR"
if [ -n "$(ls -A)" ]; then
    echo "$DIR is not empty" >&2
    exit 1
fi
mkdir old new
dl() { curl -fsSL --retry 3 -o "$2" "$1"; }

echo "== the tool"
dl "$TOOL_URL" appimageupdatetool
echo "$TOOL_SHA256  appimageupdatetool" | sha256sum -c --quiet
chmod +x appimageupdatetool

echo "== $OLD, checked against its SHA256SUMS"
dl "https://github.com/$REPO/releases/download/v$OLD/RolidSCP-$OLD-x86_64.AppImage" "RolidSCP-$OLD-x86_64.AppImage"
dl "https://github.com/$REPO/releases/download/v$OLD/SHA256SUMS" old/SHA256SUMS
sha256sum -c --ignore-missing old/SHA256SUMS
chmod +x "RolidSCP-$OLD-x86_64.AppImage"

echo "== is an update there?"
# Exit status 1 = an update is available, 0 = none (AppImageUpdate's convention).
if ./appimageupdatetool --appimage-extract-and-run --check-for-update "RolidSCP-$OLD-x86_64.AppImage"; then
    echo "AppImageUpdate finds no update for $OLD" >&2
    exit 1
fi

echo "== update"
start=$(date +%s)
./appimageupdatetool --appimage-extract-and-run "RolidSCP-$OLD-x86_64.AppImage" 2>&1 | tail -15
echo "took $(( $(date +%s) - start )) s"
OUT=RolidSCP-$NEW-x86_64.AppImage
[ -f "$OUT" ] || { echo "the update did not make $OUT:" >&2; ls -la >&2; exit 1; }

echo "== $NEW, checked against the new SHA256SUMS"
dl "https://github.com/$REPO/releases/download/v$NEW/SHA256SUMS" new/SHA256SUMS
if curl -fsSL --retry 3 -o new/SHA256SUMS.asc "https://github.com/$REPO/releases/download/v$NEW/SHA256SUMS.asc"; then
    dl "$KEY_URL" new/release-key.asc
    export GNUPGHOME=$DIR/gnupg
    mkdir -m 700 "$GNUPGHOME"
    gpg --batch --quiet --import new/release-key.asc
    gpg --batch --verify new/SHA256SUMS.asc new/SHA256SUMS
    gpgconf --kill all
else
    echo "(the release has no SHA256SUMS.asc)"
    rm -f new/SHA256SUMS.asc
fi
sha256sum -c --ignore-missing new/SHA256SUMS
[ -x "$OUT" ] || { echo "$OUT is not executable" >&2; exit 1; }
echo "== the old file is kept: $(ls -- *.zs-old 2>/dev/null || echo none)"
echo "OK: $OLD -> $NEW by AppImageUpdate, the same as the release"
