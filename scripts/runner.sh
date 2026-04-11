#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  runner.sh <host> <remote-cwd> <binary> [args...]

Description:
  Copy <binary> to <host> over SSH, mark it executable, change directory to
  <remote-cwd> on the remote host, run it remotely, forward [args...] to the
  remote binary, then remove the remote file.

Arguments:
  <host>        SSH destination, for example:
                  pi@raspberrypi
                  pi@192.168.1.10
  <remote-cwd>  Remote working directory used before execution
  <binary>      Local path to the executable built by Cargo
  [args...]     Additional arguments forwarded to the remote executable

Cargo config example:
  [target.aarch64-unknown-linux-gnu]
  linker = "aarch64-linux-gnu-gcc"
  runner = ["./scripts/runner.sh", "pi@raspberrypi", "/home/pi/project"]
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -lt 3 ]]; then
  echo "error: missing required arguments" >&2
  echo >&2
  usage >&2
  exit 2
fi

HOST="$1"
REMOTE_CWD="$2"
BIN="$3"
shift 3

BIN_NAME="$(basename "$BIN")"

ssh "$HOST" mkdir -p $REMOTE_CWD

# ここら辺にテストに必要なファイルをコピーする処理を書く
if [ -f ".env.remote" ]; then
  scp ".env.remote" "$HOST:$REMOTE_CWD/.env"
fi

scp "$BIN" "$HOST:$REMOTE_CWD/$BIN_NAME"

ssh "$HOST" bash -s -- "$REMOTE_CWD" "$BIN_NAME" "$@" <<'EOF'
set -euo pipefail
CWD="$1"
BIN_NAME="$2"
shift 2
cd "$CWD"
chmod +x "$BIN_NAME"
exec "./$BIN_NAME" "$@"
EOF
