# Forge ECS

A Rust game engine built on the Entity Component System (ECS) design pattern,
with a wgpu renderer, winit windowing, and an imgui debug UI.

---

## Architecture Diagram

```
┌────────────┐     creates      ┌───────────────┐    feeds    ┌───────────┐
│  Platform  │ ──────────────►  │ RenderContext  │ ─────────► │ DrawQueue │
│ (winit)    │                  │ (wgpu)         │            └───────────┘
└────┬───────┘                  └───────────────┘                  │
     │ PlatformEvent                                               │ flush
     │                                                             ▼
     │           ┌───────────────────────────────────────────────────────┐
     │           │                     Frame                             │
     │           │  1. scene render pass (circles + rects)               │
     │           │  2. imgui render pass (UI on top)                     │
     │           └───────────────────────────────────────────────────────┘
     │
     ▼
┌─────────────────────────────────────┐
│              ECS World              │
│  EntityAllocator  ComponentRegistry │
│  SceneTree        Resources         │
└──────────────┬──────────────────────┘
               │ &World
               ▼
         ┌──────────┐    writes to    ┌───────────────┐
         │ Systems  │ ──────────────► │ CommandBuffer │
         │ (pure fn)│                 └───────┬───────┘
         └──────────┘                         │ flush after each system
                                              ▼
                                         World mutated
```

---

## Running the demo

```bash
cargo run --bin demo
```

Requires a working GPU with Vulkan, DX12, or Metal support.

The demo opens a 1280×720 window showing two oscillating shapes:
- An **orange circle** on the left
- A **blue rectangle** on the right

The **"Entity Colors"** imgui window (top-left) lets you:
- Edit RGBA colors for each shape live
- Adjust oscillation frequency (0.1 – 5.0 Hz)
- Adjust oscillation amplitude (10 – 400 px)

---

## How to add a new Component

1. Create a plain `struct` or `enum` in `src/components/<name>.rs` deriving
   `Debug` and `Clone` — no methods, no logic.
2. Add `impl Component for YourType {}` (from `crate::ecs::component::Component`).
3. Re-export it from `src/components/mod.rs`.

---

## How to add a new System

1. Create `src/systems/<name>.rs`; define a zero-size struct and implement
   `System` for it (`fn run(&self, world: &World, commands: &mut CommandBuffer)`).
2. Write logic as iterator chains over `world.query*` — push mutations to `commands`.
3. Register it in the scheduler (`scheduler.add_system(MySystem)`) or call it
   directly in the game loop, then flush commands afterward.
