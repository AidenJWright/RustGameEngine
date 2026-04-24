//! Editor entry point — full editor UI: hierarchy, inspector, camera, save/load.
//!
//! Run with: `cargo run --bin editor`
//!
//! Controls:
//!  - Middle-drag on viewport → pan camera
//!  - Scroll wheel on viewport → zoom camera
//!  - Click entity in hierarchy → select for inspection
//!  - Inspector panel → edit Transform / Color / Sinusoid components
//!  - "Run Game" button → launches `game[.exe]` in a new window

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::too_many_lines)]

use std::f32::consts::PI;

use forge_ecs::app::EditorRunner;
use forge_ecs::components::{
    Camera, Color, Health, PlayerInput, Shape, SinusoidComponent, SpawnPoints, Tag, Transform,
    Velocity,
};
use forge_ecs::editor::{ComponentDescriptor, SystemComponentEntry};
use forge_ecs::math::Vec3;

fn main() {
    let mut runner = EditorRunner::new();
    // SinusoidSystem is intentionally NOT registered in the editor — entities
    // should remain static while editing.  It runs only in the game binary.

    // Populate the component descriptor registry used by the "Add Component"
    // dropdown.  Transform is excluded — it is guaranteed on every entity.
    runner.state.component_registry = vec![
        ComponentDescriptor {
            name: "Color",
            has: |w, e| w.get::<Color>(e).is_some(),
            add: |w, e| {
                w.insert(
                    e,
                    Color {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    },
                );
            },
            remove: |w, e| {
                w.remove::<Color>(e);
            },
        },
        ComponentDescriptor {
            name: "Shape",
            has: |w, e| w.get::<Shape>(e).is_some(),
            add: |w, e| {
                w.insert(e, Shape::Circle { radius: 50.0 });
            },
            remove: |w, e| {
                w.remove::<Shape>(e);
            },
        },
        ComponentDescriptor {
            name: "SinusoidComponent",
            has: |w, e| w.get::<SinusoidComponent>(e).is_some(),
            add: |w, e| {
                w.insert(
                    e,
                    SinusoidComponent {
                        amplitude: 100.0,
                        frequency: 1.0,
                        phase: 0.0,
                        base_y: 0.0,
                    },
                );
            },
            remove: |w, e| {
                w.remove::<SinusoidComponent>(e);
            },
        },
        ComponentDescriptor {
            name: "Velocity",
            has: |w, e| w.get::<Velocity>(e).is_some(),
            add: |w, e| {
                w.insert(e, Velocity { dx: 0.0, dy: 0.0 });
            },
            remove: |w, e| {
                w.remove::<Velocity>(e);
            },
        },
        ComponentDescriptor {
            name: "Health",
            has: |w, e| w.get::<Health>(e).is_some(),
            add: |w, e| {
                w.insert(e, Health { current: 100.0, max: 100.0 });
            },
            remove: |w, e| {
                w.remove::<Health>(e);
            },
        },
        ComponentDescriptor {
            name: "Tag",
            has: |w, e| w.get::<Tag>(e).is_some(),
            add: |w, e| {
                w.insert(e, Tag::new("entity"));
            },
            remove: |w, e| {
                w.remove::<Tag>(e);
            },
        },
        ComponentDescriptor {
            name: "Camera",
            has: |w, e| w.get::<Camera>(e).is_some(),
            add: |w, e| {
                // Enforce singleton: only add if no Camera exists in the scene.
                let already_exists = w.query::<Camera>().next().is_some();
                if !already_exists {
                    w.insert(e, Camera::new());
                }
            },
            remove: |w, e| {
                w.remove::<Camera>(e);
            },
        },
        ComponentDescriptor {
            name: "PlayerInput",
            has: |w, e| w.get::<PlayerInput>(e).is_some(),
            add: |w, e| {
                w.insert(e, PlayerInput::default());
            },
            remove: |w, e| {
                w.remove::<PlayerInput>(e);
            },
        },
        ComponentDescriptor {
            name: "SpawnPoints",
            has: |w, e| w.get::<SpawnPoints>(e).is_some(),
            add: |w, e| {
                w.insert(e, SpawnPoints::default_two());
            },
            remove: |w, e| {
                w.remove::<SpawnPoints>(e);
            },
        },
    ];

    // System-to-component map: drives the "Used by:" display in the inspector.
    runner.state.system_component_map = vec![
        SystemComponentEntry {
            system_name: "SinusoidSystem",
            component_names: &["Transform", "SinusoidComponent"],
        },
        SystemComponentEntry {
            system_name: "MovementSystem",
            component_names: &["Transform", "Velocity"],
        },
        SystemComponentEntry {
            system_name: "HealthSystem",
            component_names: &["Health"],
        },
        SystemComponentEntry {
            system_name: "PlayerInputSystem",
            component_names: &["PlayerInput", "Velocity"],
        },
        SystemComponentEntry {
            system_name: "Renderer",
            component_names: &["Transform", "Shape", "Color"],
        },
    ];

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
