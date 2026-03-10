//! Editor entry point — full editor UI: hierarchy, inspector, camera, save/load.
//!
//! Run with: `cargo run --bin editor`
//!
//! Controls:
//!  - Right-drag on viewport → pan camera
//!  - Scroll wheel on viewport → zoom camera
//!  - Click entity in hierarchy → select for inspection
//!  - Inspector panel → edit Transform / Color / Sinusoid components
//!  - "Run Game" button → launches `game[.exe]` in a new window

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::f32::consts::PI;

use forge_ecs::app::EditorRunner;
use forge_ecs::components::Transform;
use forge_ecs::components::{Color, Shape, Tag};
use forge_ecs::math::Vec3;
use forge_ecs::messaging::LoopPhase;
use forge_ecs::systems::sinusoid::SinusoidComponent;
use forge_ecs::systems::SinusoidSystem;

fn main() {
    let mut runner = EditorRunner::new();
    runner.bus.register(LoopPhase::Update, 0, SinusoidSystem);

    runner.run("Forge ECS — Editor", 1280, 720, |world| {
        let scene_root = world.spawn();
        world.insert(scene_root, Tag::new("scene_root"));

        let circle = world.spawn_child(scene_root);
        world.insert(
            circle,
            Transform {
                position: Vec3::new(-200.0, 0.0, 0.0),
                ..Transform::identity()
            },
        );
        world.insert(circle, Shape::Circle { radius: 50.0 });
        world.insert(
            circle,
            Color {
                r: 1.0,
                g: 0.4,
                b: 0.1,
                a: 1.0,
            },
        );
        world.insert(
            circle,
            SinusoidComponent {
                amplitude: 150.0,
                frequency: 1.0,
                phase: 0.0,
                base_y: 0.0,
            },
        );

        let rect = world.spawn_child(scene_root);
        world.insert(
            rect,
            Transform {
                position: Vec3::new(200.0, 0.0, 0.0),
                ..Transform::identity()
            },
        );
        world.insert(
            rect,
            Shape::Rect {
                width: 100.0,
                height: 100.0,
            },
        );
        world.insert(
            rect,
            Color {
                r: 0.2,
                g: 0.6,
                b: 1.0,
                a: 1.0,
            },
        );
        world.insert(
            rect,
            SinusoidComponent {
                amplitude: 150.0,
                frequency: 1.0,
                phase: PI / 2.0,
                base_y: 0.0,
            },
        );
    });
}
