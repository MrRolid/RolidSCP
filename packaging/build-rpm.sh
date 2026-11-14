#!/usr/bin/env bash
# Build target/rpm/rolidscp-<version>-1.x86_64.rpm from a release build.
# One package for Fedora and openSUSE: the run-time libraries are required by
# their sonames, which both distributions provide under different package names.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# rpm does not allow "-" in a version; "~" sorts a pre-release before the release.
RPM_VERSION=${VERSION//-/\~}

STAGE=target/rpm-stage
packaging/stage.sh "$STAGE"
install -Dm644 LICENSE "$STAGE/usr/share/licenses/rolidscp/LICENSE"
mv "$STAGE/usr/share/doc/rolidscp/copyright" "$STAGE/usr/share/licenses/rolidscp/copyright"

# The licence field: every licence named in the copyright file, as SPDX ids.
LICENSES=$(sed -n 's/^License: //p' "$STAGE/usr/share/licenses/rolidscp/copyright" \
    | sed 's/ and /\n/g' \
    | sed -e 's/^Apache2$/Apache-2.0/' -e 's/^Mit$/MIT/' \
          -e 's/^Bsd2Clause\(FreeBsd\)\?$/BSD-2-Clause/' -e 's/^Bsd3Clause$/BSD-3-Clause/' \
    | sort -u | grep -vx 'GPL-3.0-or-later' | paste -sd' ' | sed 's/ / AND /g')
LICENSE="GPL-3.0-or-later AND $LICENSES"

TOP=$(pwd)/target/rpm-build
rm -rf "$TOP"
mkdir -p "$TOP/SPECS" target/rpm
SPEC="$TOP/SPECS/rolidscp.spec"
cat > "$SPEC" <<SPEC
# The binary is already built and stripped by cargo (packaging/stage.sh).
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
# xz: readable by every supported rpm version.
%global _binary_payload w6.xzdio

Name:           rolidscp
Version:        $RPM_VERSION
Release:        1
Summary:        Dual-panel file manager for local disks and SFTP servers
License:        $LICENSE
URL:            https://github.com/MrRolid/RolidSCP
ExclusiveArch:  x86_64

# glibc and libgcc_s come from the automatic ELF dependencies; the graphics,
# input and font libraries are loaded at run time (dlopen).
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libX11-xcb.so.1()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libEGL.so.1()(64bit)
Requires:       libfontconfig.so.1()(64bit)
Recommends:     libvulkan.so.1()(64bit)
# A scalable system font for symbols and scripts the built-in fonts lack. The
# program starts without one too (built-in fonts only).
Recommends:     (font(notosans) or font(dejavusans) or font(liberationsans))

%description
Inspired by WinSCP and Total Commander. Color-coded tabs with SSH/SFTP
sessions, fast tab switching, quick filter, recursive copy between any
local and remote panels, strict known_hosts host key checking.

%install
cp -a "$TOP/../rpm-stage/usr" %{buildroot}/

%files
%{_bindir}/rolidscp
%{_datadir}/applications/rolidscp.desktop
%{_datadir}/icons/hicolor/scalable/apps/rolidscp.svg
%doc %{_docdir}/rolidscp
%license %{_datadir}/licenses/rolidscp
SPEC

rpmbuild -bb --quiet \
    --define "_topdir $TOP" \
    --define "_rpmdir $TOP/RPMS" \
    "$SPEC"
OUT="target/rpm/rolidscp-${RPM_VERSION}-1.x86_64.rpm"
mv "$TOP/RPMS/x86_64/rolidscp-${RPM_VERSION}-1.x86_64.rpm" "$OUT"
echo "$OUT"
