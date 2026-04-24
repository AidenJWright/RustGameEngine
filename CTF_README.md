# Capture the Flag README

This document explains where the Capture the Flag implementation lives, how to
run it, and what behavior the networked CTF mode now supports.

## Status

The networked CTF plan in `CTF_PLAN.md` is implemented.

CTF now launches through the existing multiplayer `game` flow, supports 2-4
players, uses host-selected team slots, and resolves all player entities,
flags, walls, and the camera from the saved scene file.

## File Map

### Main CTF Runtime

- `demo/game.rs`
  - Adds the CTF option to the existing matchmaker launcher.
  - Lets the host create a CTF lobby, assign players to `Red 1`, `Red 2`,
    `Blue 1`, and `Blue 2`, and start the match manually.
  - Initializes the CTF scene and networked CTF systems when a CTF match
    starts.

- `src/bin/matchmaker.rs`
  - Adds lobby support for `GameMode::CaptureTheFlag`.
  - Validates CTF slot assignments.
  - Requires 2-4 players and at least one assigned player per team before
    a CTF match can start.

- `src/multiplayer/matchmaking.rs`
  - Defines `GameMode`, `CtfSlot`, `CtfSlotAssignment`, and the matchmaker
    request/event payloads used by CTF lobbies.

### CTF Scene and Setup

- `assets/ctf_scene.json`
  - Contains the full saved CTF arena.
  - Includes the walls, divider, flags, four player entities, and the CTF
    camera entity.
  - Required CTF tags are:
    - `ctf_flag_red`
    - `ctf_flag_blue`
    - `ctf_player_red_1`
    - `ctf_player_red_2`
    - `ctf_player_blue_1`
    - `ctf_player_blue_2`
    - `ctf_camera`
    - repeated `wall`

- `src/game/ctf/setup.rs`
  - Resolves the saved scene entities by tag.
  - Records spawn/teleport locations from the saved player transforms.
  - Attaches runtime metadata components to scene-authored entities.
  - Fails loudly if a required CTF entity is missing from the scene.

### CTF Gameplay

- `src/game/ctf/mod.rs`
  - Shared CTF constants and action-bit definitions.

- `src/game/ctf/components.rs`
  - Runtime metadata components for `PlayerMarker`, `Flag`, and `Wall`.

- `src/game/ctf/resources.rs`
  - CTF resources for entity references, flag carriers, per-peer controlled
    slots, queued input frames, and immediate host snapshot sync.

- `src/game/ctf/systems/input.rs`
  - Applies network/local CTF input.
  - Handles slot switching, movement, and tagging.
  - Implements the updated tag rules:
    - defenders can tag any opponent in their side of the map
    - tagged opponents teleport back to their saved spawn
    - flagged opponents drop the carried flag before teleporting

- `src/game/ctf/systems/flag_pickup.rs`
  - Lets red slots pick up the blue flag and blue slots pick up the red flag.

- `src/game/ctf/systems/flag_carry.rs`
  - Keeps carried flags attached to the current carrier.

- `src/game/ctf/systems/wall_collision.rs`
  - Resolves player collision against the saved wall geometry.

- `src/game/ctf/systems/win_condition.rs`
  - Detects team wins when the stolen flag returns to home territory.

- `src/game/ctf/systems/hud.rs`
  - Draws the CTF HUD and victory overlay.

### Rollback and Sync

- `src/multiplayer/net_types.rs`
  - Extends snapshots with CTF-specific state.

- `src/multiplayer/rollback.rs`
  - Hashes and snapshots CTF winner state, flag carriers, and selected slots.
  - Restores that state from authoritative host corrections.

### Local Debug Runner

- `demo/ctf.rs`
  - Loads the same `assets/ctf_scene.json` scene.
  - Uses the scene-authored CTF entities instead of spawning runtime players.
  - Provides a local debug path for the CTF rules without the matchmaker flow.

## Running CTF

### Networked CTF

1. Start the matchmaker:

```sh
cargo run --bin matchmaker
```

2. Start one or more game clients:

```sh
cargo run --bin game
```

3. In the launcher:
   - choose `CaptureTheFlag`
   - create or join a lobby
   - have the host assign each connected player to a slot
   - start the match once both teams are represented

### Local Debug Runner

For a quick local gameplay check without the matchmaker UI:

```sh
cargo run --bin ctf
```

This uses the same saved CTF scene and rules, but it is a debug runner rather
than the networked match flow.

## Controls

All networked CTF clients use the same bindings:

- Move: `WASD` or arrow keys
- Tag: `Space` or `Return`
- Switch owned teammate slot: `Left Shift`

Control ownership depends on team population:

- If both team slots are occupied by humans, each player controls only their
  assigned slot.
- If a team has exactly one human, that player owns both same-team slots and
  `Left Shift` cycles between them.

## Expected Functionality

At startup:

- The CTF map loads from `assets/ctf_scene.json`.
- The saved scene provides four active player entities:
  - `Red 1` at `(160, 320)`
  - `Red 2` at `(160, 400)`
  - `Blue 1` at `(1120, 320)`
  - `Blue 2` at `(1120, 400)`
- The saved camera entity is used for CTF rendering.
- No runtime-only player, flag, wall, or camera objects are spawned for CTF.

During play:

- The game supports 2-4 players.
- The host chooses each connected player's primary CTF slot in the waiting
  room.
- Players collide with the saved wall and boundary geometry.
- Players can pick up only the opposing team's flag.
- A carrier drags the stolen flag with them.
- A defender can tag any opponent inside the defender's side of the map when
  the opponent is within tag range, whether or not the opponent is carrying a
  flag.
- A successful tag teleports the tagged player back to that slot's saved spawn
  location and clears their velocity.
- If the tagged player was carrying a flag, the flag is dropped at the tagged
  player's pre-teleport position.
- If more than one opponent is taggable, the nearest opponent is selected, with
  fixed slot-order tie breaking.
- The active controlled slot is synchronized through host-authoritative
  snapshots.

Winning:

- Red wins when any red slot returns the blue flag to red territory.
- Blue wins when any blue slot returns the red flag to blue territory.
- The HUD shows flag-carrier state and the victory overlay.
- Host snapshots replicate win state and CTF carrier state to the clients.

## Verification

These commands pass in the current repo state:

```sh
cargo test
```
