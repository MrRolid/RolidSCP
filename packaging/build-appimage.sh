#!/usr/bin/env bash
# Build target/appimage/RolidSCP-<version>-x86_64.AppImage from a release build.
#
# The AppImage holds only the program, its desktop entry, icon and licences.
# The binary links only to glibc and libgcc_s and loads the graphics, input and
# font libraries (X11, Wayland, EGL/Vulkan, fontconfig) of the host at run time,
# so none of them are bundled.
#
# Needs mksquashfs (squashfs-tools) and the AppImage type 2 runtime, downloaded
# from a fixed release and checked against its SHA-256 (or given as
# APPIMAGE_RUNTIME=<file>, which is checked the same way).
set -euo pipefail
cd "$(dirname "$0")/.."

RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
STAGE=target/appimage-stage
APPDIR=$STAGE/RolidSCP.AppDir
mkdir -p "$STAGE" target/appimage

RUNTIME=${APPIMAGE_RUNTIME:-$STAGE/runtime-x86_64}
if [ ! -f "$RUNTIME" ]; then
    curl -fsSL --retry 3 -o "$RUNTIME" "$RUNTIME_URL"
fi
echo "$RUNTIME_SHA256  $RUNTIME" | sha256sum -c --quiet

packaging/stage.sh "$APPDIR"
install -Dm644 LICENSE "$APPDIR/usr/share/licenses/rolidscp/LICENSE"
mv "$APPDIR/usr/share/doc/rolidscp/copyright" "$APPDIR/usr/share/licenses/rolidscp/copyright"
# AppImage layout: desktop entry and icon at the top, AppRun starts the program.
cp packaging/rolidscp.desktop "$APPDIR/rolidscp.desktop"
cp packaging/rolidscp.svg "$APPDIR/rolidscp.svg"
ln -s rolidscp.svg "$APPDIR/.DirIcon"
ln -s usr/bin/rolidscp "$APPDIR/AppRun"

SQUASHFS=$STAGE/rolidscp.squashfs
rm -f "$SQUASHFS"
mksquashfs "$APPDIR" "$SQUASHFS" -root-owned -noappend -comp zstd -quiet >/dev/null
OUT="target/appimage/RolidSCP-${VERSION}-x86_64.AppImage"
cat "$RUNTIME" "$SQUASHFS" > "$OUT"
chmod 755 "$OUT"
echo "$OUT"
