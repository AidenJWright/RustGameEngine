#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

MATCHMAKER_ADDR="${1:-127.0.0.1:7000}"
HOST_GAME_ADDR="${2:-127.0.0.1:7101}"
CLIENT_GAME_ADDR="${3:-127.0.0.1:7102}"
HOST_NAME="${4:-Player-One}"
CLIENT_NAME="${5:-Player-Two}"
RUN_SECONDS="${6:-120}"

HOST_LOG="$ROOT_DIR/matchmaker-host.log"
CLIENT_LOG="$ROOT_DIR/matchmaker-client.log"
MATCHMAKER_LOG="$ROOT_DIR/matchmaker-server.log"

: >"$HOST_LOG"
: >"$CLIENT_LOG"
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

  if [[ -n "${CLIENT_PID:-}" ]] && kill -0 "$CLIENT_PID" 2>/dev/null; then
    kill "$CLIENT_PID" 2>/dev/null || true
    wait "$CLIENT_PID" 2>/dev/null || true
  fi
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

echo "[4/5] starting client runtime for lobby $LOBBY_CODE"
(run_game join "$LOBBY_CODE" "$CLIENT_NAME" "$CLIENT_GAME_ADDR" >"$CLIENT_LOG" 2>&1) &
CLIENT_PID=$!

if (( RUN_SECONDS > 0 )); then
  echo "[5/5] running both clients for $RUN_SECONDS seconds"
  sleep "$RUN_SECONDS"
  echo "Demo runtime complete."
else
  echo "[5/5] running both clients; press Ctrl+C to stop"
  wait "$HOST_PID"
fi

echo "Matchmaker log: $MATCHMAKER_LOG"
echo "Host runtime log: $HOST_LOG"
echo "Client runtime log: $CLIENT_LOG"
