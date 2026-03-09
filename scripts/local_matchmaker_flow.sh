#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

PLAYER_COUNT=2
POSITIONAL_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    -p|--players)
      if [[ $# -lt 2 ]]; then
        echo "missing value for $1"
        exit 1
      fi
      PLAYER_COUNT="$2"
      shift 2
      ;;
    -h|--help)
      cat <<'USAGE'
Usage: scripts/local_matchmaker_flow.sh [--players N] [MATCHMAKER_ADDR] [HOST_GAME_ADDR] [CLIENT_GAME_ADDR] [HOST_NAME] [CLIENT_NAME] [RUN_SECONDS]

Options:
  -p, --players N    Number of game instances to launch (1-4, default: 2)

Positionals (all optional):
  MATCHMAKER_ADDR      Matchmaker bind addr (default: 127.0.0.1:7000)
  HOST_GAME_ADDR       Host game addr (default: 127.0.0.1:7101)
  CLIENT_GAME_ADDR     Base client game addr; extra clients increment port (default: 127.0.0.1:7102)
  HOST_NAME            Host player name (default: Player-One)
  CLIENT_NAME          First client name (default: Player-Two); extra clients add numeric suffixes
  RUN_SECONDS          Runtime before auto-stop, 0 means run until Ctrl+C (default: 0)
USAGE
      exit 0
      ;;
    *)
      POSITIONAL_ARGS+=("$1")
      shift
      ;;
  esac
done

if ! [[ "$PLAYER_COUNT" =~ ^[1-4]$ ]]; then
  echo "--players must be an integer in range 1-4"
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

run_game() {
  (cd "$ROOT_DIR" && cargo run --quiet --bin game -- --matchmaker "$MATCHMAKER_ADDR" "$@")
}

run_matchmaker() {
  (cd "$ROOT_DIR" && cargo run --quiet --bin matchmaker -- --bind "$MATCHMAKER_ADDR")
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

echo "[1/5] starting matchmaker on $MATCHMAKER_ADDR"
(run_matchmaker >"$MATCHMAKER_LOG" 2>&1) &
MATCHMAKER_PID=$!

sleep 1

echo "[2/5] starting host runtime"
(run_game host "$HOST_NAME" "$HOST_GAME_ADDR" >"$HOST_LOG" 2>&1) &
HOST_PID=$!

echo "[3/5] waiting for lobby code"
LOBBY_AND_ID="$(wait_for_lobby_code "$HOST_LOG" "$MATCHMAKER_LOG" "$HOST_PID")" || {
  echo "timed out waiting for host lobby creation"
  echo "host log: $HOST_LOG"
  echo "matchmaker log: $MATCHMAKER_LOG"
  exit 1
}

LOBBY_CODE="${LOBBY_AND_ID%%:*}"
HOST_ID="${LOBBY_AND_ID##*:}"
echo "host_client_id=$HOST_ID, lobby_code=$LOBBY_CODE"

if (( PLAYER_COUNT > 1 )); then
  echo "[4/5] starting $((PLAYER_COUNT - 1)) client runtime(s) for lobby $LOBBY_CODE"
  for (( i=1; i<PLAYER_COUNT; i++ )); do
    client_port=$((CLIENT_BASE_PORT + i - 1))
    client_addr="$CLIENT_HOST:$client_port"
    if (( i == 1 )); then
      client_name="$CLIENT_NAME"
    else
      client_name="$CLIENT_NAME-$((i + 1))"
    fi
    client_log="$ROOT_DIR/logs/matchmaker-client-$((i + 1)).log"
    : >"$client_log"

    (run_game join "$LOBBY_CODE" "$client_name" "$client_addr" >"$client_log" 2>&1) &
    client_pid=$!

    CLIENT_PIDS+=("$client_pid")
    CLIENT_LOGS+=("$client_log")
    echo "  started client $((i + 1)): name=$client_name addr=$client_addr log=$client_log"
  done
else
  echo "[4/5] launching host only (--players=1)"
fi

if (( RUN_SECONDS > 0 )); then
  echo "[5/5] running launched game(s) for $RUN_SECONDS seconds"
  sleep "$RUN_SECONDS"
  echo "Demo runtime complete."
else
  echo "[5/5] running launched game(s); press Ctrl+C to stop"
  wait "$HOST_PID"
fi

echo "Matchmaker log: $MATCHMAKER_LOG"
echo "Host runtime log: $HOST_LOG"
if (( ${#CLIENT_LOGS[@]} > 0 )); then
  for client_log in "${CLIENT_LOGS[@]}"; do
    echo "Client runtime log: $client_log"
  done
fi
