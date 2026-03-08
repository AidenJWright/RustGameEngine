//! **Forge ECS** — a Rust game engine built on the Entity Component System pattern.
//!
//! # Architecture
//!
//! ```text
//! Platform ──► RenderContext ──► DrawQueue ──► ImguiLayer
//!    │                                              │
//!    ▼                                              ▼
//!   ECS World ──► Systems ──► CommandBuffer ──► flush
//! ```
//!
//! ## Module overview
//! - [`ecs`] — Entity, Component, World, SceneTree, System, Resource, CommandBuffer
//! - [`math`] — Vec2, Vec3, Mat4, Transform
//! - [`components`] — game-domain plain-data components
//! - [`systems`] — pure functional systems
//! - [`platform`] — cross-platform windowing/input layer (single `winit` backend)
//! - [`renderer`] — wgpu render context, pipelines, draw queue, imgui layer
//! - [`messaging`] — `LoopPhase` messages and `MessageBus` dispatch
//! - [`scene`] — JSON scene serialisation (`save_scene` / `load_scene`)
//! - [`editor`] — `Camera2D` and `EditorState` for the editor runner
//! - [`app`] — `AppCore`, `GameRunner`, `EditorRunner`

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod app;
pub mod components;
pub mod ecs;
pub mod editor;
pub mod math;
pub mod messaging;
pub mod platform;
pub mod renderer;
pub mod scene;
pub mod systems;
