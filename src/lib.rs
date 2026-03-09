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

#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod components;
pub mod ecs;
pub mod math;
pub mod platform;
pub mod renderer;
pub mod multiplayer;
pub mod systems;
