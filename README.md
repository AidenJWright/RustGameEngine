# Forge ECS

A Rust game engine built on the Entity Component System (ECS) design pattern,
with a wgpu renderer, winit windowing, and an imgui debug UI.

---

## Architecture Diagrams

### 1) Multiplayer networking (matchmaking + peer-to-peer gameplay)

```mermaid
flowchart LR
    subgraph CLIENT_A["Game peer A (demo/game.rs)"]
        A_UI["Launcher UI"]
        A_CTRL["Control UDP socket"]
    end

    subgraph CLIENT_B["Game peer B (demo/game.rs)"]
        B_UI["Launcher UI"]
        B_CTRL["Control UDP socket"]
    end

    MM["Matchmaker (src/bin/matchmaker.rs)"]
    MATCH_STATE["MatchState\n(host_peer_id, shared_seed, player_endpoints)"]
    HOST["Host MatchSession"]
    CLIENTS["Client MatchSession(s)"]

    A_UI -->|"Ping/Create/Join/Heartbeat/Start"| A_CTRL
    B_UI -->|"Ping/Create/Join/Heartbeat/Start"| B_CTRL
    A_CTRL -->|"MatchRequest (UDP)"| MM
    B_CTRL -->|"MatchRequest (UDP)"| MM
    MM -->|"MatchEvent: LobbyCreated / LobbyJoined / LobbyUpdated / MatchStart"| A_CTRL
    MM -->|"MatchEvent: LobbyCreated / LobbyJoined / LobbyUpdated / MatchStart"| B_CTRL
    A_CTRL -->|"On MatchStart"| MATCH_STATE
    B_CTRL -->|"On MatchStart"| MATCH_STATE
    MATCH_STATE -->|"local_peer_id == host_peer_id"| HOST
    MATCH_STATE -->|"local_peer_id != host_peer_id"| CLIENTS
    CLIENTS -->|"NetMessage::Input"| HOST
    HOST -->|"Replicated NetMessage::Input"| CLIENTS
    HOST -->|"NetMessage::HostCorrection / HostHash"| CLIENTS
```

### 2) ECS update model and graphics rendering path

```mermaid
flowchart TD
    EV["winit EventLoop / ApplicationHandler"] --> UPD["about_to_wait"]
    EV --> RD["WindowEvent::RedrawRequested"]

    UPD --> RES["World resources: DeltaTime, ElapsedTime"]
    RES --> MP["Optional multiplayer tick\nInputFrame -> MatchSession::tick -> NetworkEvent"]
    RES --> BUS["MessageBus::run_frame\nFirst -> Update -> Last"]
    BUS --> CB["CommandBuffer flush after each phase"]
    CB --> WORLD["World\n(entities + components + SceneTree + resources)"]
    MP --> WORLD

    WORLD --> Q["query3<Transform, Shape, Color>"]
    Q --> DQ["DrawQueue::push(DrawCommand)"]

    RD --> BF["RenderContext::begin_frame"]
    BF --> FLUSH["DrawQueue::flush\nscene pass (circles + rects)"]
    DQ --> FLUSH
    FLUSH --> UI["Optional imgui pass\n(launcher/editor overlays)"]
    UI --> SUBMIT["wgpu queue.submit + end_frame"]
```

---

## Multiplayer demo (quick start)

```bash
# terminal 1: matchmaker (listen on all interfaces)
cargo run --bin matchmaker -- --bind 0.0.0.0:7000

# terminal 2: player client (launcher UI)
cargo run --bin game 
```
Enter 127.0.0.1:7000 as the matchmaker address and click "Connect", then proceed to create or join a lobby. 
If the client runs on another machine, replace `127.0.0.1` with the matchmaker host's LAN IP.

---

## Running the demo

```bash
cargo run --bin demo
```

Requires a working GPU with Vulkan, DX12, or Metal support.

The demo opens a 1280x720 window showing two oscillating shapes:
- An **orange circle** on the left
- A **blue rectangle** on the right

The **"Entity Colors"** imgui window (top-left) lets you:
- Edit RGBA colors for each shape live
- Adjust oscillation frequency (0.1 - 5.0 Hz)
- Adjust oscillation amplitude (10 - 400 px)
