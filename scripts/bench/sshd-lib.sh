# A throwaway OpenSSH server on 127.0.0.1 for the load tests (sourced).
#
#   start_sshd DIR   keys, config and log in DIR; sets PORT, SSHD_PID, S=DIR
#                    and ROLIDSCP_BIG_SSH (port|user|client key|known_hosts)
#
# The caller stops it with `kill "$SSHD_PID"` (the PID it was started with).
start_sshd() {
  S=$1
  mkdir -p "$S"
  ssh-keygen -q -t ed25519 -N "" -f "$S/host"
  ssh-keygen -q -t ed25519 -N "" -f "$S/client"
  cp "$S/client.pub" "$S/authorized_keys"
  PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
  cat >"$S/sshd_config" <<CFG
Port $PORT
ListenAddress 127.0.0.1
HostKey $S/host
AuthorizedKeysFile $S/authorized_keys
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
StrictModes no
PidFile $S/sshd.pid
Subsystem sftp internal-sftp
AllowUsers $(id -un)
MaxStartups 50
LogLevel INFO
CFG
  echo "[127.0.0.1]:$PORT $(cut -d' ' -f1,2 "$S/host.pub")" >"$S/known_hosts"
  /usr/sbin/sshd -D -e -f "$S/sshd_config" 2>"$S/sshd.log" &
  SSHD_PID=$!
  # Ready when it says so (a connection test could reach another program).
  for _ in $(seq 100); do grep -q "Server listening on 127.0.0.1 port $PORT" "$S/sshd.log" && break; sleep 0.1; done
  grep -q "Server listening on 127.0.0.1 port $PORT" "$S/sshd.log" || { echo "sshd did not start: $(cat "$S/sshd.log")" >&2; return 1; }
  export ROLIDSCP_BIG_SSH="$PORT|$(id -un)|$S/client|$S/known_hosts"
}
