#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PLAYER_COUNT=2
POSITIONAL_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -p|--players|-n|--num-players)
      if [[ $# -lt 2 ]]; then
        echo "missing value for $1"
        exit 1
      fi
      PLAYER_COUNT="$2"
      shift 2
      ;;
    -h|--help)
      cat <<'USAGE'
Usage: scripts/local_matchmaker_flow.sh [-n N] [MATCHMAKER_ADDR] [HOST_GAME_ADDR] [CLIENT_GAME_ADDR] [HOST_NAME] [CLIENT_NAME] [RUN_SECONDS]

Options:
  -n, --num-players N    Number of game instances to launch (default: 2)
  -p, --players N        Alias for -n

Positionals (all optional):
  MATCHMAKER_ADDR      Matchmaker address that clients connect to (default: 127.0.0.1:7000)
                       For a remote matchmaker, pass the server IP:port here, e.g. 203.0.113.5:7000
  HOST_GAME_ADDR       Host game addr (default: 127.0.0.1:7101)
  CLIENT_GAME_ADDR     Base client game addr; extra clients increment port (default: 127.0.0.1:7102)
  HOST_NAME            Host player name (default: Player-One)
  CLIENT_NAME          First client name (default: Player-Two); extra clients add numeric suffixes
  RUN_SECONDS          Runtime before auto-stop, 0 means run until Ctrl+C (default: 0)

Remote matchmaker deployment:
  On the server: MATCHMAKER_BIND=0.0.0.0:7000 cargo run --bin matchmaker
  On clients:    MATCHMAKER_ADDR=<server-ip>:7000 cargo run --bin game
  Via this script: ./scripts/local_matchmaker_flow.sh <server-ip>:7000
USAGE
      exit 0
      ;;
    *)
      POSITIONAL_ARGS+=("$1")
      shift
      ;;
  esac
done

if ! [[ "$PLAYER_COUNT" =~ ^[1-9][0-9]*$ ]]; then
  echo "-n/--num-players must be a positive integer"
  exit 1
fi

MATCHMAKER_ADDR="${POSITIONAL_ARGS[0]:-127.0.0.1:7000}"
HOST_GAME_ADDR="${POSITIONAL_ARGS[1]:-127.0.0.1:7101}"
CLIENT_GAME_ADDR="${POSITIONAL_ARGS[2]:-127.0.0.1:7102}"
HOST_NAME="${POSITIONAL_ARGS[3]:-Player-One}"
CLIENT_NAME="${POSITIONAL_ARGS[4]:-Player-Two}"
RUN_SECONDS="${POSITIONAL_ARGS[5]:-0}"

CLIENT_HOST="${CLIENT_GAME_ADDR%:*}"
CLIENT_BASE_PORT="${CLIENT_GAME_ADDR##*:}"
if [[ "$CLIENT_HOST" == "$CLIENT_GAME_ADDR" ]] || ! [[ "$CLIENT_BASE_PORT" =~ ^[0-9]+$ ]]; then
  echo "CLIENT_GAME_ADDR must be host:port, got '$CLIENT_GAME_ADDR'"
  exit 1
fi

HOST_LOG="$ROOT_DIR/logs/matchmaker-host.log"
MATCHMAKER_LOG="$ROOT_DIR/logs/matchmaker-server.log"
CLIENT_LOGS=()
CLIENT_PIDS=()

: >"$HOST_LOG"
: >"$MATCHMAKER_LOG"

# ---------------------------------------------------------------------------
# Terminal-spawning helpers.
# Each instance opens in its own terminal window so output stays separate and
# processes can be killed individually.
#
# Supported backends (auto-detected):
#   Windows : start "Title" cmd /k "..."
#   macOS   : osascript (Terminal.app)
#   Linux   : x-terminal-emulator / gnome-terminal / xterm
# ---------------------------------------------------------------------------

spawn_terminal() {
  local title="$1"
  local dir="$2"
  local cmd="$3"

  if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" || -n "${WINDIR:-}" ]]; then
    # Windows (Git Bash / MSYS2 / Cygwin)
    cmd.exe /c "start \"$title\" cmd /k \"cd /d $(cygpath -w "$dir") && $cmd\""
  elif [[ "$OSTYPE" == "darwin"* ]]; then
    osascript -e "tell application \"Terminal\" to do script \"cd '$dir' && $cmd\""
  elif command -v gnome-terminal &>/dev/null; then
    gnome-terminal --title="$title" -- bash -c "cd '$dir'; $cmd; exec bash"
  elif command -v x-terminal-emulator &>/dev/null; then
    x-terminal-emulator -T "$title" -e bash -c "cd '$dir'; $cmd; exec bash"
  elif command -v xterm &>/dev/null; then
    xterm -T "$title" -e bash -c "cd '$dir'; $cmd; exec bash" &
  else
    echo "warning: no supported terminal emulator found, running '$title' in background"
    (cd "$dir" && eval "$cmd") &
  fi
}

run_game() {
  local title="$1"
  shift
  spawn_terminal "$title" "$ROOT_DIR" \
    "cargo run --quiet --bin game -- --matchmaker \"$MATCHMAKER_ADDR\" $*"
}

run_matchmaker() {
  spawn_terminal "Matchmaker [$MATCHMAKER_ADDR]" "$ROOT_DIR" \
    "cargo run --quiet --bin matchmaker -- --bind \"$MATCHMAKER_ADDR\""
}

wait_for_lobby_code() {
  local logfile="$1"
  local matcher_logfile="$2"
  local host_pid="$3"
  local attempts=0
  local timeout=40
  while (( attempts < timeout )); do
    if ! kill -0 "$host_pid" 2>/dev/null; then
      echo "host runtime exited before lobby creation"
      return 1
    fi

    if grep -m1 "host lobby created" "$logfile" >/dev/null 2>&1; then
      local line
      line="$(grep -m1 "host lobby created" "$logfile")"

      local lobby_code
      local host_id

      if [[ "$line" =~ code=([^,]+),\ player=([0-9]+) ]]; then
        lobby_code="${BASH_REMATCH[1]}"
        host_id="${BASH_REMATCH[2]}"
        echo "$lobby_code:$host_id"
        return 0
      fi
    fi

    if grep -m1 "create-lobby" "$matcher_logfile" >/dev/null 2>&1; then
      local match_line
      local match_code
      local match_host_id
      match_line="$(grep -m1 "create-lobby" "$matcher_logfile")"

      if [[ "$match_line" =~ lobby=([^[:space:]]+) ]]; then
        match_code="${BASH_REMATCH[1]}"
      fi

      if [[ "$match_line" =~ player_id=([0-9]+) ]]; then
        match_host_id="${BASH_REMATCH[1]}"
      fi

      if [[ -n "${match_code:-}" && -n "${match_host_id:-}" ]]; then
        echo "$match_code:$match_host_id"
        return 0
      fi
    fi

    sleep 1
    attempts=$((attempts + 1))
  done

  return 1
}

cleanup() {
  if [[ -n "${MATCHMAKER_PID:-}" ]] && kill -0 "$MATCHMAKER_PID" 2>/dev/null; then
    kill "$MATCHMAKER_PID" 2>/dev/null || true
    wait "$MATCHMAKER_PID" 2>/dev/null || true
  fi

  if [[ -n "${HOST_PID:-}" ]] && kill -0 "$HOST_PID" 2>/dev/null; then
    kill "$HOST_PID" 2>/dev/null || true
    wait "$HOST_PID" 2>/dev/null || true
  fi

  local pid
  for pid in "${CLIENT_PIDS[@]:-}"; do
    if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
}

trap cleanup EXIT

echo "[1/5] starting matchmaker on $MATCHMAKER_ADDR (new terminal window)"
run_matchmaker
# Give the terminal time to open and the process to start.
sleep 2

echo "[2/5] starting host runtime (new terminal window)"
run_game "Host [$HOST_NAME]" host "$HOST_NAME" "$HOST_GAME_ADDR"
HOST_PID=$!

echo "[3/5] waiting for lobby code (check host terminal for output)"
# Give the host a moment to register the lobby with the matchmaker.
sleep 3
# Try to read lobby info from the host log if it exists, otherwise skip.
LOBBY_CODE=""
HOST_ID=""
if [[ -f "$HOST_LOG" ]]; then
  LOBBY_AND_ID="$(wait_for_lobby_code "$HOST_LOG" "$MATCHMAKER_LOG" "${HOST_PID:-0}" 2>/dev/null)" || true
  LOBBY_CODE="${LOBBY_AND_ID%%:*}"
  HOST_ID="${LOBBY_AND_ID##*:}"
fi
if [[ -n "$LOBBY_CODE" && -n "$HOST_ID" ]]; then
  echo "host_client_id=$HOST_ID, lobby_code=$LOBBY_CODE"
else
  echo "(lobby code not detected from logs — terminals handle their own output)"
  LOBBY_CODE="????"
  HOST_ID="?"
fi

if (( PLAYER_COUNT > 1 )); then
  echo "[4/5] starting $((PLAYER_COUNT - 1)) client terminal(s)"
  for (( i=1; i<PLAYER_COUNT; i++ )); do
    client_port=$((CLIENT_BASE_PORT + i - 1))
    client_addr="$CLIENT_HOST:$client_port"
    if (( i == 1 )); then
      client_name="$CLIENT_NAME"
    else
      client_name="$CLIENT_NAME-$((i + 1))"
    fi
    run_game "Client $((i + 1)) [$client_name]" join "$LOBBY_CODE" "$client_name" "$client_addr"
    echo "  opened client $((i + 1)): name=$client_name addr=$client_addr"
    sleep 1
  done
else
  echo "[4/5] launching host only (--players=1)"
fi

if (( RUN_SECONDS > 0 )); then
  echo "[5/5] all windows open; auto-stopping script in $RUN_SECONDS seconds"
  sleep "$RUN_SECONDS"
  echo "Script complete. Close individual terminal windows to stop each process."
else
  echo "[5/5] all terminal windows opened. Press Ctrl+C here to close this script."
  echo "      Each game/matchmaker runs in its own terminal — close those windows to stop them."
  # Keep script alive so the cleanup trap fires on Ctrl+C if needed.
  while true; do sleep 5; done
fi
