#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MATCHMAKER_ADDR="${1:-127.0.0.1:7000}"
HOST_GAME_ADDR="${2:-127.0.0.1:7001}"
CLIENT_GAME_ADDR="${3:-127.0.0.1:7002}"
HOST_NAME="${4:-Player-One}"
CLIENT_NAME="${5:-Player-Two}"

run_client() {
  (cd "$ROOT_DIR" && cargo run --quiet --bin matchmaker_client -- --server "$MATCHMAKER_ADDR" "$@")
}

echo "[1/4] starting matchmaker on $MATCHMAKER_ADDR"
(cd "$ROOT_DIR" && cargo run --bin matchmaker -- --bind "$MATCHMAKER_ADDR" > /tmp/matchmaker.log 2>&1) &
MATCHMAKER_PID=$!

cleanup() {
  if kill -0 "$MATCHMAKER_PID" 2>/dev/null; then
    kill "$MATCHMAKER_PID" 2>/dev/null || true
    wait "$MATCHMAKER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

sleep 1

echo "[2/4] creating lobby from host client"
HOST_OUTPUT="$(run_client create "$HOST_NAME" "$HOST_GAME_ADDR")"
echo "$HOST_OUTPUT"

LOBBY_CODE="$(echo "$HOST_OUTPUT" | awk -F= '/^lobby_code=/{print $2}')"
HOST_ID="$(echo "$HOST_OUTPUT" | awk -F= '/^client_id=/{print $2}')"

if [[ -z "$LOBBY_CODE" || -z "$HOST_ID" ]]; then
  echo "failed to create lobby; check /tmp/matchmaker.log"
  exit 1
fi

echo "[3/4] joining second client to lobby $LOBBY_CODE"
JOIN_OUTPUT="$(run_client join "$LOBBY_CODE" "$CLIENT_NAME" "$CLIENT_GAME_ADDR")"
echo "$JOIN_OUTPUT"

CLIENT_ID="$(echo "$JOIN_OUTPUT" | awk -F= '/^client_id=/{print $2}')"
if [[ -z "$CLIENT_ID" ]]; then
  echo "failed to parse client id from join response"
  exit 1
fi

echo "[4/4] sending one heartbeat from each participant"
run_client heartbeat "$LOBBY_CODE" "$HOST_ID" "$HOST_GAME_ADDR" > /dev/null || true
run_client heartbeat "$LOBBY_CODE" "$CLIENT_ID" "$CLIENT_GAME_ADDR" > /dev/null || true

echo "Connected!"
echo "lobby_code=$LOBBY_CODE"
echo "host_client_id=$HOST_ID"
echo "client_client_id=$CLIENT_ID"
echo "Matchmaker logs: /tmp/matchmaker.log"
echo "Press Ctrl+C to stop the running matchmaker process."

wait "$MATCHMAKER_PID"

