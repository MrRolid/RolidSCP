#!/usr/bin/env bash
# Build target/appimage/RolidSCP-<version>-x86_64.AppImage from a release build.
#
# The AppImage holds the program, its desktop entry, icon and licences, and the
# libraries the program opens at run time that a system may lack (keyboard,
# X11 cursor and input, Wayland cursor and EGL; the list is LIBS in
# crates/rolid-ui/src/bundled_libs.rs). They go to usr/lib/rolidscp and are
# used only where the system has no copy of its own. What the AppImage exclude
# list expects on every system (glibc, libX11, libxcb, libEGL, libGL,
# libwayland-client, fontconfig, freetype) is not bundled. The libraries are
# copied from the building system (Ubuntu 22.04 in CI, so they need no newer
# glibc than the program), with their copyright files.
#
# The runtime carries the update information for AppImageUpdate (UPDATE_INFO
# below: the newest release of MrRolid/RolidSCP on GitHub, pre-releases
# included, and its .zsync file), and the AppImage gets its .zsync file next to
# it, to be published with it. APPIMAGE_UPDATE_INFO=<text> replaces the update
# information (for tests); empty leaves it out, and the .zsync file too.
#
# Needs mksquashfs (squashfs-tools), zsyncmake (zsync), readelf (binutils) and
# the AppImage type 2 runtime, downloaded from a fixed release and checked
# against its SHA-256 (or given as APPIMAGE_RUNTIME=<file>, which is checked
# the same way).
set -euo pipefail
cd "$(dirname "$0")/.."
. packaging/reproducible.sh

RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

# "latest-all": the newest release, also when it is marked as a pre-release
# (GitHub's "latest" leaves pre-releases out, and every beta is one).
UPDATE_INFO=${APPIMAGE_UPDATE_INFO-"gh-releases-zsync|MrRolid|RolidSCP|latest-all|RolidSCP-*-x86_64.AppImage.zsync"}

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

# The libraries for systems that lack them (see the top of this file).
LIBDIR=$APPDIR/usr/lib/rolidscp
mkdir -p "$LIBDIR" "$APPDIR/usr/share/licenses/rolidscp/bundled"
# An empty keyboard data folder for systems without one (bundled_libs::prepare_env).
mkdir -p "$APPDIR/usr/share/rolidscp/xkb"
for lib in $(sed -n '/^pub const LIBS/,/^];/p' crates/rolid-ui/src/bundled_libs.rs | grep -oE 'lib[A-Za-z0-9_-]+\.so\.[0-9]+'); do
    src=$(/sbin/ldconfig -p | awk -v l="$lib" '$1 == l && /x86-64/ { print $NF; exit }')
    if [ -z "$src" ]; then
        echo "missing on this system: $lib (install it to build the AppImage)" >&2
        exit 1
    fi
    cp -L "$src" "$LIBDIR/$lib"
    chmod 644 "$LIBDIR/$lib"
    pkg=$(dpkg -S "$(readlink -f "$src")" 2>/dev/null | head -1 | cut -d: -f1)
    if [ -n "$pkg" ] && [ -f "/usr/share/doc/$pkg/copyright" ]; then
        cp "/usr/share/doc/$pkg/copyright" "$APPDIR/usr/share/licenses/rolidscp/bundled/$pkg.copyright"
        # The mode of the system's copy differs between runner images.
        chmod 644 "$APPDIR/usr/share/licenses/rolidscp/bundled/$pkg.copyright"
    else
        echo "no copyright file for $lib" >&2
        exit 1
    fi
    # With the version: a security update of the system package between two
    # builds changes the AppImage (the reproducibility check then says why).
    echo "bundled $lib ($pkg $(dpkg-query -W -f '${Version}' "$pkg" 2>/dev/null))"
done

SQUASHFS=$STAGE/rolidscp.squashfs
rm -f "$SQUASHFS"
clamp_times "$APPDIR"
# SOURCE_DATE_EPOCH sets the times inside; extended attributes of the build
# machine's files stay out.
mksquashfs "$APPDIR" "$SQUASHFS" -root-owned -no-xattrs -noappend -comp zstd -quiet >/dev/null
OUT="target/appimage/RolidSCP-${VERSION}-x86_64.AppImage"
rm -f "$OUT" "$OUT.zsync"
# The update information goes into the runtime's .upd_info section (as
# appimagetool -u does), in a copy of the runtime.
RT=$STAGE/runtime-with-update-info
cp "$RUNTIME" "$RT"
if [ -n "$UPDATE_INFO" ]; then
    # "[23] .upd_info PROGBITS <address> <offset> <size> ...", in hex.
    read -r off size < <(readelf -S -W "$RT" | sed -n 's/.*\] \.upd_info  *[A-Z]*  *[0-9a-f]*  *\([0-9a-f]*\)  *\([0-9a-f]*\) .*/\1 \2/p')
    if [ -z "${off:-}" ] || [ "${#UPDATE_INFO}" -ge "$((16#$size))" ]; then
        echo "the runtime has no .upd_info section, or the update information is too long" >&2
        exit 1
    fi
    printf '%s' "$UPDATE_INFO" | dd of="$RT" bs=1 seek="$((16#$off))" conv=notrunc status=none
fi
cat "$RT" "$SQUASHFS" > "$OUT"
chmod 755 "$OUT"
# zsyncmake writes the file's time into the .zsync file.
touch -d "@$SOURCE_DATE_EPOCH" "$OUT"
echo "$OUT"
if [ -n "$UPDATE_INFO" ]; then
    # The AppImage's address in the .zsync file is its bare name: the tools
    # look for it next to the .zsync file (on the release page).
    (cd target/appimage && zsyncmake -u "$(basename "$OUT")" -o "$(basename "$OUT").zsync" "$(basename "$OUT")" >/dev/null)
    echo "$OUT.zsync"
fi
