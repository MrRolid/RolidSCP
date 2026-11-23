#!/bin/bash
# Run a command inside an unpacked distribution (scripts/compat/rootfs.sh) as
# its root, without being root: a private user and mount namespace (only the
# own user is mapped, to 0), the host's /dev, /proc, /sys and network, and a
# chroot. Package managers work in it (a few chown calls of their scripts
# fail and are ignored).
#
#   scripts/compat/inroot.sh ROOTFS [-n] [-u UID] [-b HOSTDIR[:INSIDE]]... -- COMMAND...
#
# -b binds a host folder into the tree (default: the same path inside).
# -n gives the command no network (only a loopback that is down).
# -u runs the command as user UID of the tree instead of its root (servers
#    that do not run as root let only their own user log in).
# Everything runs in its own PID namespace: stopping this script by its PID
# ends all processes started inside.
set -euo pipefail
ROOTFS=$(realpath "$1"); shift
BINDS=()
AS=(-r)
NET=()
while [ $# -gt 0 ] && [ "$1" != -- ]; do
  case $1 in
    -b) BINDS+=("$2"); shift 2 ;;
    -n) NET=(--net); shift ;;
    -u) AS=(--user --map-user="$2" --map-group="$2" --keep-caps); shift 2 ;;
    *) echo "unknown $1" >&2; exit 2 ;;
  esac
done
shift
[ $# -gt 0 ] || { sed -n '2,10p' "$0"; exit 2; }
cp -L /etc/resolv.conf "$ROOTFS/etc/resolv.conf" 2>/dev/null || true
exec unshare "${AS[@]}" "${NET[@]}" --mount --pid --fork --kill-child bash -c '
  set -e
  R=$1; shift
  n=$1; shift
  for d in dev sys; do mkdir -p "$R/$d"; mount --rbind "/$d" "$R/$d"; done
  mkdir -p "$R/proc"; mount -t proc proc "$R/proc"
  mkdir -p "$R/tmp/.X11-unix"
  [ -d /tmp/.X11-unix ] && mount --bind /tmp/.X11-unix "$R/tmp/.X11-unix"
  for ((i = 0; i < n; i++)); do
    b=$1; shift
    host=${b%%:*}; inside=${b#*:}; [ "$inside" = "$b" ] && inside=$host
    mkdir -p "$R$inside"; mount --bind "$host" "$R$inside"
  done
  exec /usr/sbin/chroot "$R" /usr/bin/env -i PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin HOME=/root TERM=dumb LANG=C.UTF-8 "$@"
' _ "$ROOTFS" "${#BINDS[@]}" "${BINDS[@]}" "$@"
