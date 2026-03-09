# Multiplayer plan for Forge ECS (Renet, randomized host, host-of-truth reconciliation)

## Summary
- Add a **three-layer multiplayer architecture**:  
  1) external matchmaker control plane, 2) gameplay transport in peer-to-peer sessions (host is a normal peer), 3) deterministic ECS input replication with host snapshots for desync recovery.
- Keep the existing single-player demo path intact and add multiplayer as an optional path in `demo/main.rs`.
- Use host-selected snapshots only as authoritative truth when divergence is detected, matching your requirement.

## Key changes

- Add Renet dependencies and serialization types in `[dependencies]` of [Cargo.toml](/home/rakom/Documents/CSUN/680/RustGameEngine/Cargo.toml), plus transport defaults (UDP config, channels, tick settings) in module-level constants.
- Add a new `src/multiplayer` module with:
- `src/multiplayer/mod.rs` to wire transport/session primitives.
- `src/multiplayer/matchmaking.rs` for discovery/lobby handoff APIs (no gameplay traffic).
- `src/multiplayer/net_types.rs` for `MatchState`, `NetMessage`, `InputFrame`, `Snapshot`, `EntityStatePacket`, and channel identifiers.
- `src/multiplayer/session.rs` for session lifecycle, connection setup, peer role assignment, and tick loop coordination.
- `src/multiplayer/rollback.rs` for authoritative hash checks and host-state correction application.

- Host selection:
- Use a deterministic host election during matchmaking setup using shared seed + participant list ordering.
- Persist selected host in session state so all peers agree before game start.
- Non-host peers establish P2P links directly to host (not through relay traffic for gameplay packets).

- Matchmaking and transport:
- Keep matchmaking as client/server-only control plane:
- create or join a lobby,
- receive `match_id`, `shared_seed`, `participants`, `host_endpoint`, and optional NAT hint.
- Establish gameplay transport by connecting every peer directly to host endpoint.

- ECS integration strategy (input replication):
- Introduce a new `InputState` resource that stores per-tick local command intent (e.g., movement/actions encoded as compact deterministic commands).
- During `Event::AboutToWait`, sample local input and enqueue an `InputFrame` for local tick only.
- Renet sends `InputFrame` from non-host to host on reliable/ordered control channel.
- Host processes all input frames for tick `t`, advances world, emits optional local authoritative `FrameHash`/full `Snapshot` every N ticks.
- Clients simulate locally for responsiveness and apply host correction when a snapshot is tagged as correction or hash mismatch.

- Desync handling:
- Add rolling state hash per tick on host and clients.
- On mismatch, host sends minimal authoritative correction packet: either full snapshot or entity delta from last confirmed tick.
- Non-host applies correction by replacing stale state and replaying buffered local inputs up to current tick (if rollback window is available).

- Engine-facing API additions:
- Add multiplayer-facing world hooks in [src/lib.rs](/home/rakom/Documents/CSUN/680/RustGameEngine/src/lib.rs):  
  `forge_ecs::multiplayer::{MultiplayerConfig, MatchSession, NetworkTick, PlayerInputFrame, NetworkEvent}`.
- Add optional frame scheduler entry points:
  - `NetworkInputSystem` to serialize local input.
  - `NetworkApplySystem` to apply local/remote authority corrections.
- Keep existing single-player `System`/`CommandBuffer` flow unchanged when multiplayer is disabled.

- Demo integration:
- Update [demo/main.rs](/home/rakom/Documents/CSUN/680/RustGameEngine/demo/main.rs) with:
- mode flag (`--single` vs `--host`/`--join` or env-driven toggle),
- matchmaking bootstrap,
- per-frame net tick send/receive boundaries around `SinusoidSystem` and input handling.
- Keep current rendering and ImGui UI as-is for parity.

## Public interfaces and behavior contracts

- `MatchmakingClient::create_or_join(match_id, player_id)`  
  returns lobby state with deterministic host seed and peer endpoints.
- `MatchSession::is_host()`, `MatchSession::local_peer_id()`, `MatchSession::players()`.
- `MatchSession::tick()`  
  drives send/receive + local simulation advancement.
- `NetMessage::Input(InputFrame)` and `NetMessage::HostCorrection(Snapshot|Delta, tick)`.
- `NetworkPolicy` flags:
  - `sync_mode: InputReplication`
  - `desync_mode: SnapshotCorrections`
  - `tick_rate: u32`

## Rollout and test plan

- Unit tests:
- deterministic host election with identical participant list and seed across peers,
- input frame ordering/replay buffer behavior,
- checksum mismatch detection and corrective packet parsing.
- Integration tests:
- two-process local loopback game start: host + one client inputs produce synchronized transforms within tolerance,
- disconnect/reconnect flow where host remains authoritative,
- forced desync injection => host correction applied and simulation converges.
- Manual validation:
- run demo in host/join modes and verify host changes drive both peers, with non-host local UI still responsive.

## Assumptions/defaults chosen

- Use `input replication` as primary sync mode, with host snapshots used for correction only on hash mismatch.
- Start with UDP transport via Renet default channels and a single authoritative host in each room.
- Fixed step clock at 60 Hz for multiplayer determinism.
- Snapshot/correction sends every 5 ticks by default, reduced only when mismatch risk is high.
- Initial game scope is generalized lobby/session architecture so the engine can support more than one scene later. 
