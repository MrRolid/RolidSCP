#!/bin/bash
# Servers of different kinds against the program's own code (roadmap 0.8.18):
# OpenSSH of several versions, Dropbear, an account with SFTP only, BusyBox
# tools and a chroot SFTP account, each started without root (OpenSSH and
# Dropbear of other systems run in an unpacked image of that system, see
# rootfs.sh and inroot.sh). The test `bench_servers::compat_server` then
# connects, lists, transfers both ways with SHA-256, uses F4, permissions,
# rename, delete, archives and the server tools (crates/rolid-ui/src/bench_servers.rs).
#
#   scripts/compat/servers.sh WORK RESULTS.tsv [SERVER...]
#
# WORK     a folder for the images and keys (about 1.5 GB; deleted at the end)
# SERVER   names from SERVERS below (default: all)
#
# Nothing is written to ~/.ssh: keys and known_hosts are in WORK. Every
# server listens on 127.0.0.1 only and is stopped by the PID it was started with.
set -euo pipefail
[ $# -ge 2 ] || { sed -n '2,16p' "$0"; exit 2; }
WORK=$(realpath -m "$1"); OUT=$(realpath -m "$2"); shift 2
HERE=$(dirname "$(realpath "$0")")
ROOT=$(realpath "$HERE/../..")
CARGO=${CARGO:-cargo}
mkdir -p "$WORK"

# name | image ("-" = this computer) | kind
SERVERS=(
  "openssh-7.6|ubuntu:18.04|openssh"
  "openssh-8.2|ubuntu:20.04|openssh"
  "openssh-9.2|debian:12|openssh"
  "openssh-9.6|ubuntu:24.04|openssh"
  "openssh-host|-|openssh"
  "sftp-only|-|sftponly"
  "chroot-sftp|debian:12|chroot"
  "dropbear|debian:12|dropbear"
  "busybox-openssh|alpine:3.22|openssh"
  "busybox-dropbear|alpine:3.22|dropbear"
)
WANT=("$@")

note() { echo "== $(date +%T) $*" >&2; }
# The server running now: stopped by its PID whatever happens.
SPID=
stop_server() {
  if [ -n "$SPID" ]; then
    # unshare --fork does not end on TERM: KILL after a moment (its
    # --kill-child then ends everything inside).
    kill "$SPID" 2>/dev/null || true
    for _ in $(seq 25); do kill -0 "$SPID" 2>/dev/null || break; sleep 0.2; done
    kill -KILL "$SPID" 2>/dev/null || true
    wait "$SPID" 2>/dev/null || true
  fi
  SPID=
}
trap stop_server EXIT
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }

echo "== building" >&2
TEST_BIN=$(cd "$ROOT" && "$CARGO" test --release -p rolidscp --no-run --message-format=json 2>/dev/null |
  python3 -c 'import json,sys
for l in sys.stdin:
    m = json.loads(l)
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["kind"] == ["bin"] and m["profile"]["test"]:
        print(m["executable"])')

# The packages each image needs.
install() {
  local rootfs=$1 kind=$2
  case $(cat "$rootfs/etc/os-release") in
    *ID=alpine*)
      "$HERE/inroot.sh" "$rootfs" -- sh -c "apk add -q --no-cache openssh-server openssh-sftp-server dropbear dropbear-ssh >/dev/null 2>&1 || true" ;;
    *)
      local pkgs="openssh-server openssh-sftp-server"
      [ "$kind" = dropbear ] && pkgs="dropbear-bin openssh-sftp-server"
      "$HERE/inroot.sh" "$rootfs" -- bash -c "echo 'APT::Sandbox::User \"root\";' >/etc/apt/apt.conf.d/99sandbox
        apt-get update -qq >/dev/null 2>&1; DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends $pkgs >/var/log/compat-install.log 2>&1 || true" ;;
  esac
}

# A user with the uid of this computer's user inside the image (servers that
# do not run as root let only their own user in).
add_user() {
  local rootfs=$1
  local name
  name=$(awk -F: '$3 == 1000 {print $1}' "$rootfs/etc/passwd")
  if [ -z "$name" ]; then
    name=tester
    echo "tester:x:1000:1000::/home/tester:/bin/sh" >>"$rootfs/etc/passwd"
    echo "tester:x:1000:" >>"$rootfs/etc/group"
    grep -q '^tester:' "$rootfs/etc/shadow" 2>/dev/null || echo "tester:*:19000:0:99999:7:::" >>"$rootfs/etc/shadow"
  fi
  # A locked account ("!" in shadow) may not log in even with a key.
  sed -i "s/^$name:![^:]*:/$name:*:/" "$rootfs/etc/shadow" 2>/dev/null || true
  local home
  home=$(awk -F: -v n="$name" '$1 == n {print $6}' "$rootfs/etc/passwd")
  mkdir -p "$rootfs$home/.ssh"
  # Dropbear refuses a home folder that others can write to.
  chmod 755 "$rootfs$home"; chmod 700 "$rootfs$home/.ssh"
  echo "$name $home"
}

run_test() {
  local name=$1 port=$2 user=$3 key=$4 kh=$5 remote=${6:-}
  note "$name: testing on port $port"
  set +e
  env ROLIDSCP_COMPAT_SSH="127.0.0.1|$port|$user|$key|$kh" ROLIDSCP_COMPAT_TAG="$name" ROLIDSCP_COMPAT_LOCAL="$WORK/local-$name" \
    ${remote:+ROLIDSCP_COMPAT_REMOTE="$remote"} \
    timeout 600 "$TEST_BIN" --ignored --exact bench_servers::compat_server --test-threads=1 --nocapture >"$WORK/$name.log" 2>&1
  local rc=$?
  set -e
  grep "^COMPAT" "$WORK/$name.log" | tee -a "$OUT" || true
  echo -e "COMPAT\t$name\tpassed\t$([ $rc = 0 ] && echo ok || echo fail)\texit $rc" | tee -a "$OUT"
  [ $rc = 0 ] || tail -5 "$WORK/$name.log" >&2
  cp "$WORK/$name.log" "${OUT%.tsv}-$name.log"
}

wait_port() {
  local port=$1
  for _ in $(seq 100); do
    (echo >"/dev/tcp/127.0.0.1/$port") 2>/dev/null && return 0
    sleep 0.2
  done
  return 1
}

: >"$OUT"
for s in "${SERVERS[@]}"; do
  IFS='|' read -r name image kind <<<"$s"
  if [ ${#WANT[@]} -gt 0 ] && [[ " ${WANT[*]} " != *" $name "* ]]; then continue; fi
  K="$WORK/keys-$name"
  rm -rf "$K"; mkdir -p "$K"
  ssh-keygen -q -t ed25519 -N "" -f "$K/client"
  port=$(free_port)

  if [ "$image" = - ]; then
    # This computer's OpenSSH, as the normal user.
    ssh-keygen -q -t ed25519 -N "" -f "$K/host"
    cp "$K/client.pub" "$K/authorized_keys"
    extra=""
    [ "$kind" = sftponly ] && extra="ForceCommand internal-sftp
DisableForwarding yes"
    cat >"$K/sshd_config" <<CFG
Port $port
ListenAddress 127.0.0.1
HostKey $K/host
AuthorizedKeysFile $K/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
PidFile $K/sshd.pid
Subsystem sftp internal-sftp
$extra
CFG
    echo "[127.0.0.1]:$port $(cut -d' ' -f1,2 "$K/host.pub")" >"$K/known_hosts"
    note "$name: starting $(/usr/sbin/sshd -V 2>&1 | head -1)"
    /usr/sbin/sshd -D -e -f "$K/sshd_config" 2>"$K/sshd.log" &
    SPID=$!
    if wait_port "$port"; then
      mkdir -p "$WORK/remote-$name"
      run_test "$name" "$port" "$(id -un)" "$K/client" "$K/known_hosts" "$WORK/remote-$name/rolidscp-compat"
    else
      echo -e "COMPAT\t$name\tstart\tfail\t$(tail -3 "$K/sshd.log" | tr '\n' ' ')" | tee -a "$OUT"
    fi
    stop_server
    continue
  fi

  R="$WORK/rootfs-${image//[:\/]/-}"
  if [ ! -e "$R/.compat-$kind" ]; then
    [ -e "$R" ] || "$HERE/rootfs.sh" "$image" "$R"
    note "$name: installing the server into $image"
    install "$R" "$kind"
    touch "$R/.compat-$kind"
  fi
  read -r user home <<<"$(add_user "$R")"
  mkdir -p "$R/compat"
  rm -rf "$R/compat/$name"; mkdir -p "$R/compat/$name"
  C="/compat/$name"
  remote=""
  case $kind in
    openssh|chroot)
      ssh-keygen -q -t ed25519 -N "" -f "$R$C/host"
      cp "$K/client.pub" "$R$C/authorized_keys"
      sftp=internal-sftp
      extra=""
      if [ "$kind" = chroot ]; then
        # The account sees only its jail (owned by root, as sshd demands).
        mkdir -p "$R/jail/upload"
        extra="ChrootDirectory /jail
ForceCommand internal-sftp -d /upload"
        remote=/upload/rolidscp-compat
      fi
      cat >"$R$C/sshd_config" <<CFG
Port $port
ListenAddress 127.0.0.1
HostKey $C/host
AuthorizedKeysFile $C/authorized_keys
PasswordAuthentication no
UsePAM no
StrictModes no
PidFile $C/sshd.pid
Subsystem sftp $sftp
$extra
CFG
      echo "[127.0.0.1]:$port $(cut -d' ' -f1,2 "$R$C/host.pub")" >"$K/known_hosts"
      if [ "$kind" = chroot ]; then
        # sshd must run as root to chroot: as root of this namespace, and
        # only root is mapped there.
        cmd="/usr/sbin/sshd -D -e -f $C/sshd_config"
        as=()
        user=root
        chmod 755 "$R/jail"
        # Its helper user for the separated, unprivileged part (only root exists here).
        grep -q '^sshd:' "$R/etc/passwd" || echo "sshd:x:0:0:privsep:/run/sshd:/usr/sbin/nologin" >>"$R/etc/passwd"
        mkdir -p "$R/run/sshd"; chmod 755 "$R/run/sshd"
        # setgroups() is refused in a user namespace: a stand-in for this sshd only.
        cc -shared -fPIC -O2 -o "$R$C/nosetgroups.so" "$HERE/nosetgroups.c"
        cmd="env LD_PRELOAD=$C/nosetgroups.so $cmd"
      else
        cmd="/usr/sbin/sshd -D -e -f $C/sshd_config"
        as=(-u 1000)
      fi
      ver=$("$HERE/inroot.sh" "$R" -- /usr/sbin/sshd -V 2>&1 | grep -o 'OpenSSH_[^ ,]*' | head -1 || true)
      ;;
    dropbear)
      "$HERE/inroot.sh" "$R" -- dropbearkey -t ed25519 -f "$C/host" >/dev/null 2>&1
      "$HERE/inroot.sh" "$R" -- dropbearkey -y -f "$C/host" 2>/dev/null | grep '^ssh-' | cut -d' ' -f1,2 |
        sed "s/^/[127.0.0.1]:$port /" >"$K/known_hosts"
      cp "$K/client.pub" "$R$home/.ssh/authorized_keys"
      chmod 700 "$R$home/.ssh"; chmod 600 "$R$home/.ssh/authorized_keys"
      cmd="dropbear -F -E -s -p 127.0.0.1:$port -r $C/host -P $C/db.pid"
      as=(-u 1000)
      ver=$("$HERE/inroot.sh" "$R" -- dropbear -V 2>&1 | head -1 || true)
      ;;
  esac
  note "$name: starting $ver in $image"
  "$HERE/inroot.sh" "$R" "${as[@]}" -- sh -c "$cmd" >"$K/server.log" 2>&1 &
  SPID=$!
  if wait_port "$port"; then
    echo -e "COMPAT\t$name\tserver\tinfo\t$ver ($image)" | tee -a "$OUT"
    run_test "$name" "$port" "$user" "$K/client" "$K/known_hosts" "$remote"
  else
    echo -e "COMPAT\t$name\tstart\tfail\t$(tail -3 "$K/server.log" | tr '\n' ' ')" | tee -a "$OUT"
  fi
  stop_server
done
note "done"
