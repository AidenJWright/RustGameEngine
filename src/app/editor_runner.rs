//! `EditorRunner` — full editor UI: hierarchy, inspector, camera, save/load, run game.

use std::f32::consts::PI;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::components::{Color, Shape, Tag, Transform};
use crate::ecs::entity::Entity;
use crate::ecs::resource::{DeltaTime, ElapsedTime};
use crate::ecs::world::World;
use crate::editor::EditorState;
use crate::messaging::MessageBus;
use crate::renderer::draw::DrawCommand;
use crate::scene::{load_scene, save_scene};
use crate::systems::sinusoid::SinusoidComponent;

use super::core::AppCore;
use super::game_runner::make_draw_cmd;

/// Full editor loop: 2D camera viewport, hierarchy panel, component inspector,
/// scene save/load, and a "Run Game" button that spawns the game binary.
pub struct EditorRunner {
    /// Message bus — register systems to run in the background while editing.
    pub bus: MessageBus,
    /// Editor UI state.
    pub state: EditorState,
    last_time: Instant,
}

impl EditorRunner {
    /// Create a runner with an empty message bus and default editor state.
    pub fn new() -> Self {
        Self {
            bus: MessageBus::new(),
            state: EditorState::default(),
            last_time: Instant::now(),
        }
    }

    /// Create the event loop, build the window inside `resumed`, and block
    /// until the window closes.
    pub fn run(
        self,
        title: &str,
        width: u32,
        height: u32,
        setup: impl FnOnce(&mut World) + 'static,
    ) {
        let event_loop = EventLoop::new().expect("failed to create event loop");
        let mut handler = EditorHandle {
            runner: self,
            setup: Some(Box::new(setup)),
            title: title.to_string(),
            width,
            height,
            core: None,
        };
        event_loop.run_app(&mut handler).expect("event loop error");
    }

    // -----------------------------------------------------------------------

    fn update(&mut self, core: &mut AppCore) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_time).as_secs_f32();
        self.last_time = now;

        if let Some(r) = core.world.resource_mut::<DeltaTime>() {
            r.0 = dt;
        }
        if let Some(r) = core.world.resource_mut::<ElapsedTime>() {
            r.0 += dt;
        }

        self.bus.run_frame(&mut core.world);
    }

    #[allow(clippy::too_many_lines)]
    fn render(&mut self, core: &mut AppCore) {
        core.render_ctx.sync_with_window(core.platform.window());

        let w = core.render_ctx.surface_config.width as f32;
        let h = core.render_ctx.surface_config.height as f32;

        // --- 1. Build scene draw-commands with camera transform ---
        let draw_cmds: Vec<DrawCommand> = core
            .world
            .query3::<Transform, Shape, Color>()
            .map(|(_, t, s, c)| {
                let raw = make_draw_cmd(t, s, c); // world-origin coords
                self.state.camera.transform_draw_cmd(raw, w, h)
            })
            .collect();
        for cmd in draw_cmds {
            core.draw_queue.push(cmd);
        }

        // --- 2. Collect hierarchy data (before imgui borrows world) ---
        let hierarchy: Vec<(Entity, String)> = {
            let mut items = Vec::new();
            let roots: Vec<Entity> = core.world.scene_tree().root_entities().collect();
            for root in roots {
                core.world.scene_tree().walk_depth_first(root, |e, depth| {
                    let indent = "  ".repeat(depth);
                    let label = core
                        .world
                        .get::<Tag>(e)
                        .map(|t| format!("{indent}{}", t.as_str()))
                        .unwrap_or_else(|| format!("{indent}{e}"));
                    items.push((e, label));
                });
            }
            items
        };

        // --- 3. Clone component data for inspector (avoids mid-UI borrows) ---
        let selected = self.state.selected_entity;
        let (mut new_transform, mut new_color, mut new_sinusoid) = if let Some(e) = selected {
            (
                core.world.get::<Transform>(e).cloned(),
                core.world.get::<Color>(e).cloned(),
                core.world.get::<SinusoidComponent>(e).cloned(),
            )
        } else {
            (None, None, None)
        };

        // --- 4. Action flags collected during UI ---
        let mut new_selected = selected;
        let mut transform_changed = false;
        let mut color_changed = false;
        let mut sinusoid_changed = false;
        let mut remove_transform = false;
        let mut remove_color = false;
        let mut remove_sinusoid = false;
        let mut spawn_req = false;
        let mut despawn_req = false;
        let mut run_game_req = false;
        let mut save_req = false;
        let mut load_req = false;
        let mut cam_pan = [0.0_f32; 2];
        let mut cam_zoom = 0.0_f32;

        // --- 5. Begin GPU frame ---
        let Some((surface_texture, view)) = core.render_ctx.begin_frame() else {
            return;
        };
        let mut encoder =
            core.render_ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("editor frame"),
                });

        core.draw_queue.flush(
            &core.render_ctx,
            &view,
            &mut encoder,
            &core.circle_pipeline,
            &core.rect_pipeline,
            [0.08, 0.08, 0.12, 1.0],
        );

        // --- 6. imgui UI (NLL ensures ui borrow ends before end_frame) ---
        {
            let ui = core.imgui.begin_frame(&core.platform.window);

            // Toolbar
            ui.window("##toolbar")
                .no_decoration()
                .size([w, 30.0], imgui::Condition::Always)
                .position([0.0, 0.0], imgui::Condition::Always)
                .build(|| {
                    if ui.button("Save") {
                        save_req = true;
                    }
                    ui.same_line();
                    if ui.button("Load") {
                        load_req = true;
                    }
                    ui.same_line();
                    if ui.button("Run Game") {
                        run_game_req = true;
                    }
                    ui.same_line();
                    if ui.button("+ Spawn") {
                        spawn_req = true;
                    }
                    ui.same_line();
                    ui.text(format!("  Scene: {}", self.state.scene_path));
                });

            // Hierarchy panel
            ui.window("Scene Hierarchy")
                .size([210.0, h - 55.0], imgui::Condition::Always)
                .position([0.0, 30.0], imgui::Condition::Always)
                .build(|| {
                    for (entity, label) in &hierarchy {
                        let is_sel = new_selected == Some(*entity);
                        let prefix = if is_sel { "> " } else { "  " };
                        let btn_label = format!("{prefix}{label}##{entity:?}");
                        if ui.button(&btn_label) {
                            new_selected = Some(*entity);
                        }
                    }
                });

            // Inspector panel
            if selected.is_some() {
                ui.window("Inspector")
                    .size([280.0, h - 55.0], imgui::Condition::Always)
                    .position([w - 290.0, 30.0], imgui::Condition::Always)
                    .build(|| {
                        if let Some(entity) = selected {
                            ui.text(format!("{entity}"));
                            ui.separator();
                        }

                        // --- Transform ---
                        if let Some(ref mut tf) = new_transform {
                            ui.text("[ Transform ]");
                            if ui.input_float("px##tf", &mut tf.position.x).build() {
                                transform_changed = true;
                            }
                            if ui.input_float("py##tf", &mut tf.position.y).build() {
                                transform_changed = true;
                            }
                            if ui.input_float("rot##tf", &mut tf.rotation).build() {
                                transform_changed = true;
                            }
                            if ui.input_float("sx##tf", &mut tf.scale.x).build() {
                                transform_changed = true;
                            }
                            if ui.input_float("sy##tf", &mut tf.scale.y).build() {
                                transform_changed = true;
                            }
                            if ui.small_button("Remove##rm_tf") {
                                remove_transform = true;
                            }
                            ui.separator();
                        } else if ui.button("+ Transform") {
                            new_transform = Some(Transform::identity());
                            transform_changed = true;
                        }

                        // --- Color ---
                        if let Some(ref mut c) = new_color {
                            ui.text("[ Color ]");
                            let mut arr = [c.r, c.g, c.b, c.a];
                            if ui.color_edit4("##col", &mut arr) {
                                c.r = arr[0];
                                c.g = arr[1];
                                c.b = arr[2];
                                c.a = arr[3];
                                color_changed = true;
                            }
                            if ui.small_button("Remove##rm_col") {
                                remove_color = true;
                            }
                            ui.separator();
                        } else if ui.button("+ Color") {
                            new_color = Some(Color {
                                r: 1.0,
                                g: 1.0,
                                b: 1.0,
                                a: 1.0,
                            });
                            color_changed = true;
                        }

                        // --- Sinusoid ---
                        if let Some(ref mut sin) = new_sinusoid {
                            ui.text("[ Sinusoid ]");
                            if ui.slider("amp##sin", 0.0_f32, 500.0, &mut sin.amplitude) {
                                sinusoid_changed = true;
                            }
                            if ui.slider("freq##sin", 0.0_f32, 10.0, &mut sin.frequency) {
                                sinusoid_changed = true;
                            }
                            if ui.slider("phase##sin", -PI, PI, &mut sin.phase) {
                                sinusoid_changed = true;
                            }
                            if ui.slider("base_y##sin", -400.0_f32, 400.0, &mut sin.base_y) {
                                sinusoid_changed = true;
                            }
                            if ui.small_button("Remove##rm_sin") {
                                remove_sinusoid = true;
                            }
                            ui.separator();
                        } else if ui.button("+ Sinusoid") {
                            new_sinusoid = Some(SinusoidComponent {
                                amplitude: 100.0,
                                frequency: 1.0,
                                phase: 0.0,
                                base_y: 0.0,
                            });
                            sinusoid_changed = true;
                        }

                        ui.separator();
                        if ui.button("Despawn Entity") {
                            despawn_req = true;
                        }
                    });
            }

            // Camera control via imgui IO (right-drag to pan, scroll to zoom)
            {
                let io = ui.io();
                if io.mouse_down[1] && !ui.is_any_item_active() {
                    cam_pan = io.mouse_delta;
                }
                let wheel = io.mouse_wheel;
                if wheel != 0.0 {
                    cam_zoom = wheel;
                }
            }

            // Status bar
            if !self.state.status_message.is_empty() {
                let msg = self.state.status_message.clone();
                ui.window("##status")
                    .no_decoration()
                    .size([w, 22.0], imgui::Condition::Always)
                    .position([0.0, h - 22.0], imgui::Condition::Always)
                    .build(|| {
                        ui.text(&msg);
                    });
            }
        } // ui dropped here — NLL releases borrow of core.imgui

        core.imgui.end_frame(
            &core.platform.window,
            &core.render_ctx.device,
            &core.render_ctx.queue,
            &mut encoder,
            &view,
        );

        core.render_ctx
            .queue
            .submit(std::iter::once(encoder.finish()));
        core.render_ctx.end_frame(surface_texture);

        // --- 7. Apply state changes collected during UI ---
        self.state.selected_entity = new_selected;
        self.state.camera.pan(cam_pan[0], cam_pan[1]);
        if cam_zoom != 0.0 {
            self.state.camera.zoom_toward(cam_zoom);
        }

        if let Some(entity) = selected {
            if remove_transform {
                core.world.remove::<Transform>(entity);
            } else if transform_changed {
                if let Some(tf) = new_transform {
                    core.world.insert(entity, tf);
                }
            }

            if remove_color {
                core.world.remove::<Color>(entity);
            } else if color_changed {
                if let Some(c) = new_color {
                    core.world.insert(entity, c);
                }
            }

            if remove_sinusoid {
                core.world.remove::<SinusoidComponent>(entity);
            } else if sinusoid_changed {
                if let Some(s) = new_sinusoid {
                    core.world.insert(entity, s);
                }
            }
        }

        if spawn_req {
            let e = core.world.spawn();
            core.world.insert(e, Transform::identity());
            self.state.selected_entity = Some(e);
        }

        if despawn_req {
            if let Some(entity) = selected {
                core.world.despawn(entity);
                self.state.selected_entity = None;
            }
        }

        if run_game_req {
            run_game(&mut self.state.status_message);
        }

        if save_req {
            match save_scene(&core.world, &self.state.scene_path) {
                Ok(()) => self.state.status_message = format!("Saved → {}", self.state.scene_path),
                Err(e) => self.state.status_message = format!("Save error: {e}"),
            }
        }

        if load_req {
            match load_scene(&mut core.world, &self.state.scene_path) {
                Ok(()) => self.state.status_message = format!("Loaded ← {}", self.state.scene_path),
                Err(e) => self.state.status_message = format!("Load error: {e}"),
            }
        }
    }
}

impl Default for EditorRunner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// ApplicationHandler impl
// ---------------------------------------------------------------------------

struct EditorHandle {
    runner: EditorRunner,
    setup: Option<Box<dyn FnOnce(&mut World)>>,
    title: String,
    width: u32,
    height: u32,
    core: Option<AppCore>,
}

impl ApplicationHandler for EditorHandle {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.core.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title(&self.title)
            .with_inner_size(PhysicalSize::new(self.width, self.height))
            .with_resizable(true);
        let window: Window = event_loop
            .create_window(attrs)
            .expect("window creation failed");
        let mut core = AppCore::from_window(window).expect("AppCore creation failed");
        if let Some(setup) = self.setup.take() {
            setup(&mut core.world);
        }
        self.core = Some(core);
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let Some(core) = &mut self.core {
            let full = winit::event::Event::<()>::NewEvents(cause);
            core.imgui.handle_event(core.platform.window(), &full);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(core) = &mut self.core else {
            return;
        };
        if window_id != core.platform.window.id() {
            return;
        }

        core.imgui
            .handle_window_event(core.platform.window(), window_id, &event);

        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(s) => core.render_ctx.resize(s.width, s.height),
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = core.platform.window.inner_size();
                core.render_ctx.resize(size.width, size.height);
            }
            WindowEvent::RedrawRequested => self.runner.render(core),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        let Some(core) = &mut self.core else {
            return;
        };
        core.imgui.handle_about_to_wait(core.platform.window());
        self.runner.update(core);
        core.platform.window.request_redraw();
    }
}

// ---------------------------------------------------------------------------
// Helper: launch the game binary
// ---------------------------------------------------------------------------

fn run_game(status: &mut String) {
    // Try to find the `game[.exe]` binary next to the current executable.
    let game_bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .map(|dir| {
            if cfg!(windows) {
                dir.join("game.exe")
            } else {
                dir.join("game")
            }
        });

    let launched = game_bin
        .as_ref()
        .filter(|p| p.exists())
        .and_then(|p| std::process::Command::new(p).spawn().ok())
        .is_some();

    if launched {
        *status = "Game launched.".to_string();
    } else {
        // Fallback: try cargo run.
        let spawned = std::process::Command::new("cargo")
            .args(["run", "--bin", "game"])
            .spawn()
            .is_ok();
        *status = if spawned {
            "Launching game via cargo run...".to_string()
        } else {
            "Could not launch game binary.".to_string()
        };
    }
}
