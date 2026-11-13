#!/usr/bin/env bash
# Install a built package in a clean container (run as root inside it) and
# start the installed program with smoke-test.sh.
#
# Usage: test-package.sh <deb|rpm> <package file> <expected version>
# The repository is the current directory.
set -euo pipefail
KIND=${1:?usage: test-package.sh <deb|rpm> <package> <version>}
PKG=${2:?missing package file}
EXPECT=${3:?missing version}

. /etc/os-release
echo "== $PRETTY_NAME: installing $PKG"
case "$KIND" in
deb)
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -q
    # The package with its own dependencies and recommendations, plus the test tools.
    apt-get install -y -q "./$PKG" xvfb x11-utils
    ;;
rpm)
    if command -v dnf >/dev/null; then
        dnf install -y -q "./$PKG" xorg-x11-server-Xvfb xwininfo
    else
        # The package is not signed; zypper refuses that without the switch.
        zypper --non-interactive install --allow-unsigned-rpm "./$PKG" xorg-x11-server-Xvfb xwininfo
    fi
    ;;
*)
    echo "unknown package kind $KIND" >&2
    exit 2
    ;;
esac

# Desktop entry, icon and licence files are where the menus and tools look for them.
for f in /usr/bin/rolidscp /usr/share/applications/rolidscp.desktop \
         /usr/share/icons/hicolor/scalable/apps/rolidscp.svg; do
    test -e "$f" || { echo "FAIL: missing $f" >&2; exit 1; }
done
if [ "$KIND" = deb ]; then
    test -s /usr/share/doc/rolidscp/copyright || { echo "FAIL: missing copyright" >&2; exit 1; }
else
    test -s /usr/share/licenses/rolidscp/LICENSE || { echo "FAIL: missing LICENSE" >&2; exit 1; }
fi

# Every library the binary needs is present.
if command -v ldd >/dev/null && ldd /usr/bin/rolidscp | grep -q 'not found'; then
    ldd /usr/bin/rolidscp >&2
    echo "FAIL: missing libraries" >&2
    exit 1
fi

bash packaging/ci/smoke-test.sh "$EXPECT" rolidscp
