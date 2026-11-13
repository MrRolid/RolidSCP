#!/usr/bin/env bash
# Build target/debian/rolidscp_<version>_<arch>.deb from a release build.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(dpkg --print-architecture)
cargo build --release -p rolidscp

# Staging outside target/debian (that folder only collects the packages).
STAGE=target/deb-stage
rm -rf "$STAGE"
install -Dm755 target/release/rolidscp "$STAGE/usr/bin/rolidscp"
install -Dm644 packaging/rolidscp.desktop "$STAGE/usr/share/applications/rolidscp.desktop"
install -Dm644 packaging/rolidscp.svg "$STAGE/usr/share/icons/hicolor/scalable/apps/rolidscp.svg"
install -Dm644 README.md "$STAGE/usr/share/doc/rolidscp/README.md"
install -Dm644 docs/README.cs.md "$STAGE/usr/share/doc/rolidscp/README.cs.md"
COPYRIGHT="$STAGE/usr/share/doc/rolidscp/copyright"
cat > "$COPYRIGHT" <<COPY
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: rolidscp
Source: https://github.com/MrRolid/RolidSCP

Files: *
Copyright: 2026 Martin Vorel <martin@vorel.eu>
License: GPL-3.0-or-later
 This program is free software: you can redistribute it and/or modify
 it under the terms of the GNU General Public License as published by
 the Free Software Foundation, either version 3 of the License, or
 (at your option) any later version.
 .
 This program is distributed in the hope that it will be useful,
 but WITHOUT ANY WARRANTY; without even the implied warranty of
 MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 GNU General Public License for more details.
 .
 On Debian systems, the complete text of the GNU General Public License
 version 3 can be found in "/usr/share/common-licenses/GPL-3".

Files: crates/rolid-ui/assets/phosphor/*
Copyright: 2023 Phosphor Icons
License: MIT
COPY
sed 's/^$/./; s/^/ /' crates/rolid-ui/assets/phosphor/LICENSE >> "$COPYRIGHT"
printf '\nFiles: crates/rolid-ui/assets/fonts/NotoSans-Regular.ttf\nCopyright: 2015 Google LLC\nComment: Noto Sans Regular 2.004, the built-in text font\nLicense: OFL-1.1\n' >> "$COPYRIGHT"
sed 's/^$/./; s/^/ /' crates/rolid-ui/assets/fonts/OFL.txt >> "$COPYRIGHT"
printf '\nFiles: crates/rolid-ui/assets/fonts/Hack-Regular.ttf\nCopyright: 2018 Source Foundry Authors\n 2003 Bitstream, Inc.\nComment: Hack 3.003, the built-in monospace font (dates, sizes)\nLicense: MIT and Bitstream-Vera\n' >> "$COPYRIGHT"
sed 's/^$/./; s/^/ /' crates/rolid-ui/assets/fonts/Hack-LICENSE.txt >> "$COPYRIGHT"
# Licence notices of all libraries built into the binary, made fresh on every build.
if ! command -v cargo-about >/dev/null; then
    echo "cargo-about is missing: cargo install cargo-about --locked --features cli" >&2
    exit 1
fi
ABOUT_JSON=$(mktemp)
trap 'rm -f "$ABOUT_JSON"' EXIT
cargo-about generate --locked -c packaging/about/about.toml --format json -o "$ABOUT_JSON"
python3 packaging/about/copyright.py "$ABOUT_JSON" >> "$COPYRIGHT"
# Licences of the syntax definitions built into the viewer (F3).
cargo run -q --release -p rolid-view --example syntax_licenses >> "$COPYRIGHT"

SIZE=$(du -sk "$STAGE/usr" | cut -f1)
mkdir -p "$STAGE/DEBIAN" target/debian
cat > "$STAGE/DEBIAN/control" <<CTRL
Package: rolidscp
Version: $VERSION
Section: utils
Priority: optional
Architecture: $ARCH
Installed-Size: $SIZE
Maintainer: Martin Vorel <martin@vorel.eu>
Depends: libc6 (>= 2.39), libgcc-s1, libxkbcommon0, libxkbcommon-x11-0, libx11-6, libx11-xcb1, libxcursor1, libxi6, libwayland-client0, libwayland-egl1, libegl1, libfontconfig1
Recommends: libvulkan1, mesa-vulkan-drivers | nvidia-vulkan-icd, libegl-mesa0
Description: Dual-panel file manager for local disks and SFTP servers
 Inspired by WinSCP and Total Commander. Color-coded tabs with SSH/SFTP
 sessions, fast tab switching, quick filter, recursive copy between any
 local and remote panels, strict known_hosts host key checking.
CTRL

OUT="target/debian/rolidscp_${VERSION}_${ARCH}.deb"
fakeroot dpkg-deb --build -Zxz "$STAGE" "$OUT" >/dev/null
echo "$OUT"
