#!/usr/bin/env bash
# Build target/debian/rolidscp_<version>_<arch>.deb from a release build.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# "~" sorts a pre-release (0.9.0-beta.1) before the release, as in rpm.
DEB_VERSION=${VERSION//-/\~}
ARCH=$(dpkg --print-architecture)

# Staging outside target/debian (that folder only collects the packages).
STAGE=target/deb-stage
packaging/stage.sh "$STAGE"

# The binary links only to glibc and libgcc_s; the newest glibc symbol version
# it uses is the lowest libc6 it runs on (so it follows the build machine).
GLIBC=$(objdump -T "$STAGE/usr/bin/rolidscp" | grep -o 'GLIBC_[0-9][0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -1)
[ -n "$GLIBC" ] || { echo "no glibc symbol versions found" >&2; exit 1; }

SIZE=$(du -sk "$STAGE/usr" | cut -f1)
mkdir -p "$STAGE/DEBIAN" target/debian
# The graphics, input and font libraries are loaded at run time (dlopen),
# so they are listed by hand.
cat > "$STAGE/DEBIAN/control" <<CTRL
Package: rolidscp
Version: $DEB_VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $SIZE
Maintainer: Martin Vorel <martin@vorel.eu>
Homepage: https://github.com/MrRolid/RolidSCP
Depends: libc6 (>= $GLIBC), libgcc-s1, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libx11-xcb1, libxcursor1, libxi6, libwayland-client0, libwayland-egl1, libegl1, libfontconfig1
Recommends: libvulkan1, mesa-vulkan-drivers | nvidia-vulkan-icd, libegl-mesa0
Description: Dual-panel file manager for local disks and SFTP servers
 Inspired by WinSCP and Total Commander. Color-coded tabs with SSH/SFTP
 sessions, fast tab switching, quick filter, recursive copy between any
 local and remote panels, strict known_hosts host key checking.
CTRL

OUT="target/debian/rolidscp_${DEB_VERSION}_${ARCH}.deb"
fakeroot dpkg-deb --build -Zxz "$STAGE" "$OUT" >/dev/null
echo "$OUT"
