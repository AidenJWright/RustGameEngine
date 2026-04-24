# Capture the Flag README

This document maps the CTF implementation to the files in this repository,
explains how to run it, and describes the expected gameplay behavior.

## Implementation Status

All gameplay features from `CTF_PLAN.md` are implemented.

Two implementation details differ slightly from the priority table in the plan:

- `WallCollisionSystem` runs after `MovementSystem` so it can resolve the
  position players just moved into.
- The HUD is drawn from the CTF render path instead of being registered as a
  `LoopPhase::Last` ECS system, because it needs the current ImGui frame.

## File Map

### Game Entry Point

- `demo/ctf.rs`
  - Creates the fixed 1280 x 720 CTF window.
  - Loads `assets/ctf_scene.json`.
  - Spawns both triangle players at the scene spawn markers.
  - Adds runtime-only `Flag`, `Wall`, and `PlayerMarker` components.
  - Registers the CTF gameplay systems.
  - Handles CTF keyboard input and draws the CTF HUD.

### CTF Map

- `assets/ctf_scene.json`
  - Contains the static arena geometry.
  - Includes both colored zones, the divider, base pockets, mid-field walls,
    boundary walls, flag entities, and spawn marker entities.
  - Required tags include `p1_flag`, `p2_flag`, `p1_spawn`, `p2_spawn`, and
    repeated `wall` tags for collision geometry.

### CTF Components, Resources, and Systems

- `src/game/ctf/mod.rs`
  - Shared CTF constants such as arena size, player radius, flag radius,
    tag range, player speed, and tag key discriminants.

- `src/game/ctf/components.rs`
  - Runtime-only CTF components:
    - `PlayerMarker`
    - `Flag`
    - `Wall`

- `src/game/ctf/resources.rs`
  - CTF singleton resources:
    - `GameState`
    - `GamePhase`
    - `EntityRefs`
    - `CarrierState`
    - `TagInputState`

- `src/game/ctf/systems/wall_collision.rs`
  - Pushes player circles out of wall AABBs.

- `src/game/ctf/systems/flag_pickup.rs`
  - Lets players pick up only the opponent flag.

- `src/game/ctf/systems/flag_carry.rs`
  - Keeps a carried flag visually offset under its carrier.

- `src/game/ctf/systems/tagging.rs`
  - Handles edge-triggered defensive tags, flag drops, and carrier teleports.

- `src/game/ctf/systems/win_condition.rs`
  - Detects wins and stops player movement after the match ends.

- `src/game/ctf/systems/hud.rs`
  - Draws the CTF status HUD and victory overlay.

### Engine Support Used by CTF

- `Cargo.toml`
  - Defines the `ctf` binary.

- `src/components/shape.rs`
  - Adds `Shape::Triangle`.

- `src/renderer/draw.rs`
  - Adds `DrawCommand::Triangle` and flushes triangle draw commands.

- `src/renderer/pipeline.rs`
  - Adds `TrianglePipeline` with a rotated triangle SDF shader.

- `src/app/core.rs`
  - Owns the triangle pipeline alongside circle and rect pipelines.

- `src/app/game_runner.rs`, `demo/game.rs`, and `demo/main.rs`
  - Convert `Shape::Triangle` into triangle draw commands for existing runners.

- `src/app/editor_runner.rs`
  - Adds Triangle to the editor's Shape type dropdown.

- `src/platform/winit.rs`, `src/components/player_input.rs`, and
  `src/systems/player_input.rs`
  - Map Return and other CTF keys into the engine input system.

## Running the CTF Game

Run from the repository root:

```sh
cargo run --bin ctf
```

The game opens a non-resizable 1280 x 720 window titled
`Forge ECS -- Capture the Flag`.

## Controls

| Player | Move | Tag |
| --- | --- | --- |
| Player 1, red | WASD | Space |
| Player 2, blue | Arrow keys | Return |

## Expected Functionality

At startup:

- A red left territory, blue right territory, center divider, walls, base
  pockets, flags, and boundary walls are visible.
- Player 1 spawns as a red triangle at `(160, 360)`.
- Player 2 spawns as a blue triangle at `(1120, 360)`.

During play:

- Both players can move at the same time.
- Players collide with all `wall` entities and cannot leave the arena.
- Player 1 can pick up only Player 2's blue flag.
- Player 2 can pick up only Player 1's red flag.
- A carried flag follows below the carrier so it remains visible.
- Player 1 can tag Player 2 only when Player 2 carries the red flag on
  Player 1's side, is within tag range, and Player 1 presses Space.
- Player 2 can tag Player 1 only when Player 1 carries the blue flag on
  Player 2's side, is within tag range, and Player 2 presses Return.
- A successful tag drops the stolen flag at the carrier's current position
  and teleports the carrier back to their spawn.
- Tag actions are edge-triggered, so holding the tag key does not repeatedly
  fire tags.

Winning:

- Player 1 wins by carrying the blue flag back to the left side of the map.
- Player 2 wins by carrying the red flag back to the right side of the map.
- A victory overlay appears when a player wins.
- Player movement stops after the game ends.
- Restart by closing the window and running `cargo run --bin ctf` again.

## Verification

These commands were run successfully:

```sh
cargo build --bin ctf
cargo test --bin ctf
cargo test
```
