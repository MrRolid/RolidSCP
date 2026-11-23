#!/bin/bash
# The release packages installed and used in other distributions, without
# docker or root: each distribution is unpacked from its image (rootfs.sh),
# the package goes in through the distribution's own package manager (with
# what it recommends, as a user would install it), and gui-scenario.sh then
# drives the program's window inside it (Xvfb and the SSH server run on this
# computer). The AppImage is run in every distribution as well.
#
#   scripts/compat/distros.sh PKGDIR WORK OUT [NAME...]
#
# PKGDIR  the release files (rolidscp_<v>_amd64.deb, rolidscp-<v>-1.x86_64.rpm,
#         RolidSCP-<v>-x86_64.AppImage)
# WORK    a scratch folder (an unpacked distribution is removed after its run)
# OUT     a TSV file the results are added to
# NAME    only these distributions (default: all)
set -euo pipefail
[ $# -ge 3 ] || { sed -n '2,15p' "$0"; exit 2; }
PKG=$(realpath "$1"); WORK=$(realpath -m "$2"); OUT=$(realpath -m "$3"); shift 3
HERE=$(dirname "$(realpath "$0")")
DEB=$(ls "$PKG"/rolidscp_*_amd64.deb); RPM=$(ls "$PKG"/rolidscp-*.x86_64.rpm); APPIMAGE=$(ls "$PKG"/RolidSCP-*-x86_64.AppImage)
mkdir -p "$WORK"
note() { echo "== $(date +%T) $*" >&2; }

# name|image|package
DISTROS=(
  "debian-12|debian:12|deb"
  "debian-13|debian:13|deb"
  "ubuntu-22.04|ubuntu:22.04|deb"
  "ubuntu-24.04|ubuntu:24.04|deb"
  "mint-22|linuxmintd/mint22-amd64:latest|deb"
  "fedora-42|fedora:42|rpm"
  "fedora-43|fedora:43|rpm"
  "opensuse-leap-16|opensuse/leap:16.0|rpm"
  "opensuse-tumbleweed|opensuse/tumbleweed:latest|rpm"
  "arch|archlinux:latest|appimage"
)

# The commands that install the package (inside the distribution as its root).
install_cmd() {
  case $1 in
    deb) echo "echo 'APT::Sandbox::User \"root\";' >/etc/apt/apt.conf.d/99sandbox
          apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq /pkg/$(basename "$DEB")
          dpkg-query -W rolidscp" ;;
    rpm) if [[ $2 == opensuse* ]]; then
           echo "zypper -n --gpg-auto-import-keys install --allow-unsigned-rpm /pkg/$(basename "$RPM") && rpm -q rolidscp"
         else
           echo "dnf -y -q install /pkg/$(basename "$RPM") && rpm -q rolidscp"
         fi ;;
    appimage) # A desktop has a graphics driver; this image has nothing.
      echo "sed -i 's/^CheckSpace/#CheckSpace/' /etc/pacman.conf
            pacman -Sy --noconfirm --disable-sandbox --needed mesa fontconfig ttf-dejavu >/dev/null && pacman -Q mesa" ;;
  esac
}

for entry in "${DISTROS[@]}"; do
  IFS='|' read -r name image kind <<<"$entry"
  if [ $# -gt 0 ] && [[ " $* " != *" $name "* ]]; then continue; fi
  R=$WORK/root-$name
  rm -rf "$R" "$WORK/gui-$name" "$WORK/gui-$name-appimage"
  note "$name: unpacking $image"
  if ! "$HERE/rootfs.sh" "$image" "$R" >"$WORK/$name.log" 2>&1; then
    echo -e "DISTRO\t$name\tunpack\tfail\t$(tail -1 "$WORK/$name.log")" | tee -a "$OUT"; continue
  fi
  os=$(. "$R/etc/os-release" 2>/dev/null; echo "${PRETTY_NAME:-?}")
  # Linux Mint's image is its Ubuntu base with Mint's repositories.
  mint=$(grep -rhoE 'packages\.linuxmint\.com +[a-z]+' "$R/etc/apt/sources.list" "$R/etc/apt/sources.list.d" 2>/dev/null | awk '{ print $2; exit }' || true)
  [ -n "$mint" ] && os="$os with the Linux Mint repositories ($mint)"
  echo -e "DISTRO\t$name\tsystem\tinfo\t$os" | tee -a "$OUT"
  note "$name: installing the $kind package"
  if out=$("$HERE/inroot.sh" "$R" -b "$PKG:/pkg" -- bash -c "$(install_cmd "$kind" "$name")" 2>&1 | tee -a "$WORK/$name.log" | tail -1); then
    echo -e "DISTRO\t$name\tinstall $kind\tok\t$out" | tee -a "$OUT"
  else
    echo -e "DISTRO\t$name\tinstall $kind\tfail\t$out" | tee -a "$OUT"
  fi
  runs=()
  [ "$kind" != appimage ] && runs+=("$kind")
  runs+=(appimage)
  for run in "${runs[@]}"; do
    G=$WORK/gui-$name-$run
    note "$name: the window ($run)"
    if [ "$run" = appimage ]; then
      # Without FUSE in the distribution the AppImage unpacks itself and runs.
      mkdir -p "$G/app/tmp"; cp "$APPIMAGE" "$G/app/RolidSCP.AppImage"; chmod +x "$G/app/RolidSCP.AppImage"
      printf '#!/bin/sh\nAPPIMAGE_EXTRACT_AND_RUN=1 TMPDIR=%s exec %s "$@"\n' "$G/app/tmp" "$G/app/RolidSCP.AppImage" >"$G/app/rolidscp"
      chmod +x "$G/app/rolidscp"
      launch=("$HERE/inroot.sh" "$R" -b "$G" --)
      app_path="$G/app:/usr/bin:/bin"
    else
      launch=("$HERE/inroot.sh" "$R" -b "$G" --)
      app_path=/usr/bin:/bin
    fi
    rm -rf "$G/w"
    rc=0
    APP_PATH=$app_path timeout 900 "$HERE/gui-scenario.sh" "$G/w" "$G/shots" -- "${launch[@]}" >"$G.out" 2>&1 || rc=$?
    sed "s/^GUI\t/DISTRO\t$name\t$run: /; s/^LOG\t/DISTRO\t$name\t$run: log\tinfo\t/" "$G.out" | grep "^DISTRO" | tee -a "$OUT" || true
    echo -e "DISTRO\t$name\t$run: passed\t$([ $rc = 0 ] && echo ok || echo fail)\texit $rc" | tee -a "$OUT"
    rm -rf "$G/w" "$G/app"
  done
  rm -rf "$R"
done
note done
