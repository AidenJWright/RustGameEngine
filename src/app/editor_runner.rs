//! `EditorRunner` — full editor UI: hierarchy, inspector, camera, save/load, run game.

use std::f32::consts::PI;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::KeyLocation;
use winit::window::{Window, WindowAttributes, WindowId};

use crate::components::{
    Color, ConfigKey, PlayerInput, Shape, SinusoidComponent, SpawnPoints, Tag, Transform,
};
use crate::ecs::entity::Entity;
use crate::ecs::resource::{DeltaTime, ElapsedTime};
use crate::ecs::world::World;
use crate::editor::state::{
    EditorWindowRect, PlayerInputBinding, PlayerInputCapture, SceneEntityDrag,
};
use crate::editor::{EditorState, SystemComponentEntry};
use crate::math::Vec2;
use crate::messaging::MessageBus;
use crate::platform::{map_physical_key, KeyCode};
use crate::renderer::draw::DrawCommand;
#[cfg(feature = "file-dialog")]
use crate::scene::{reload_scene, save_scene};
use crate::systems::player_input::config_key_from_key_code;
use imgui::{MouseButton, WindowHoveredFlags};
#[cfg(feature = "file-dialog")]
use rfd;

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

        let surface_w = core.render_ctx.surface_config.width as f32;
        let surface_h = core.render_ctx.surface_config.height as f32;

        // The editor UI is a transparent overlay, so the scene uses the full surface.
        let viewport_w = surface_w.max(1.0);
        let viewport_h = surface_h.max(1.0);

        // --- 1. Build scene draw-commands with camera transform ---
        let draw_cmds: Vec<DrawCommand> = core
            .world
            .query3::<Transform, Shape, Color>()
            .map(|(_, t, s, c)| {
                let raw = make_draw_cmd(t, s, c); // world-origin coords
                self.state
                    .camera
                    .transform_draw_cmd(raw, viewport_w, viewport_h)
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
        let mut new_tag = selected
            .and_then(|e| core.world.get::<Tag>(e).map(|t| t.0.clone()))
            .unwrap_or_default();
        let (
            mut new_transform,
            mut new_color,
            mut new_shape,
            mut new_sinusoid,
            mut new_player_input,
            mut new_spawn_points,
        ) = if let Some(e) = selected {
            (
                core.world.get::<Transform>(e).cloned(),
                core.world.get::<Color>(e).cloned(),
                core.world.get::<Shape>(e).cloned(),
                core.world.get::<SinusoidComponent>(e).cloned(),
                core.world.get::<PlayerInput>(e).cloned(),
                core.world.get::<SpawnPoints>(e).cloned(),
            )
        } else {
            (None, None, None, None, None, None)
        };

        // --- 4. Action flags collected during UI ---
        let mut new_selected = selected;
        let mut tag_changed = false;
        let mut transform_changed = false;
        let mut color_changed = false;
        let mut shape_changed = false;
        let mut sinusoid_changed = false;
        let mut player_input_changed = false;
        let mut spawn_points_changed = false;
        let mut remove_color = false;
        let mut remove_shape = false;
        let mut remove_sinusoid = false;
        let mut remove_player_input = false;
        let mut remove_spawn_points = false;
        let mut spawn_req = false;
        let mut despawn_req = false;
        let mut run_game_req = false;
        let mut save_req = false;
        let mut load_req = false;
        let mut cam_pan = [0.0_f32; 2];
        let mut cam_zoom = 0.0_f32;
        // Index into component_registry to add after the UI pass; None = no add.
        let mut add_component_req: Option<usize> = None;
        // Drag-and-drop reparent request: (child, new_parent).
        let mut reparent_req: Option<(Entity, Entity)> = None;
        // Make dragged entity a root (detach from parent).
        let mut detach_req: Option<Entity> = None;
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
            &core.triangle_pipeline,
            [0.08, 0.08, 0.12, 1.0],
        );

        // --- 6. imgui UI (NLL ensures ui borrow ends before end_frame) ---
        {
            let ui = core.imgui.begin_frame(&core.platform.window);

            // Use ImGui's logical display size for layout/docking math (important on HiDPI).
            let display_w = ui.io().display_size[0];
            let display_h = ui.io().display_size[1];

            let editor_panel_bg_alpha = 0.62_f32;
            let default_editor_window_pos = [16.0, 16.0];
            let default_editor_window_size = [520.0, (display_h - 32.0).min(560.0).max(240.0)];
            let restoring_editor_window =
                self.state.editor_window_restore_pending && !self.state.editor_window_maximized;
            let (editor_window_pos, editor_window_size, editor_window_condition) =
                if self.state.editor_window_maximized {
                    ([0.0, 0.0], [display_w, display_h], imgui::Condition::Always)
                } else if restoring_editor_window {
                    let rect = self
                        .state
                        .editor_window_restore_rect
                        .unwrap_or(EditorWindowRect {
                            pos: default_editor_window_pos,
                            size: default_editor_window_size,
                        });
                    (rect.pos, rect.size, imgui::Condition::Always)
                } else {
                    (
                        default_editor_window_pos,
                        default_editor_window_size,
                        imgui::Condition::FirstUseEver,
                    )
                };
            let mut editor_window_flags =
                imgui::WindowFlags::NO_SAVED_SETTINGS | imgui::WindowFlags::NO_DOCKING;
            if self.state.editor_window_maximized {
                editor_window_flags |= imgui::WindowFlags::NO_MOVE | imgui::WindowFlags::NO_RESIZE;
            }

            ui.window("Editor")
                .bg_alpha(editor_panel_bg_alpha)
                .position(editor_window_pos, editor_window_condition)
                .size(editor_window_size, editor_window_condition)
                .size_constraints([360.0, 240.0], [display_w.max(360.0), display_h.max(240.0)])
                .flags(editor_window_flags)
                .build(|| {
                    if restoring_editor_window {
                        self.state.editor_window_restore_pending = false;
                    }

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
                    let maximize_label = if self.state.editor_window_maximized {
                        "Restore"
                    } else {
                        "Maximize"
                    };
                    if ui.button(maximize_label) {
                        if self.state.editor_window_maximized {
                            self.state.editor_window_maximized = false;
                            self.state.editor_window_restore_pending = true;
                        } else {
                            self.state.editor_window_restore_rect = Some(EditorWindowRect {
                                pos: ui.window_pos(),
                                size: ui.window_size(),
                            });
                            self.state.editor_window_maximized = true;
                            self.state.editor_window_restore_pending = false;
                        }
                    }
                    ui.same_line();
                    ui.text(format!("  Scene: {}", self.state.scene_path));
                    if !self.state.status_message.is_empty() {
                        ui.separator();
                        ui.text(&self.state.status_message);
                    }
                    ui.separator();

                    if let Some(_tabs) = ui.tab_bar("##editor_tabs") {
                        if let Some(_tab) = ui.tab_item("Scene Hierarchy") {
                            for (entity, label) in &hierarchy {
                                let is_sel = new_selected == Some(*entity);
                                let prefix = if is_sel { "> " } else { "  " };
                                let btn_label = format!("{prefix}{label}##{entity:?}");
                                if ui.button(&btn_label) {
                                    new_selected = Some(*entity);
                                }

                                // --- Drag source: grab this entity ---
                                // Encode entity as u64 (index | generation<<32).
                                let drag_id =
                                    (entity.index as u64) | ((entity.generation as u64) << 32);
                                if let Some(src) = ui
                                    .drag_drop_source_config("entity_drag")
                                    .begin_payload(drag_id)
                                {
                                    ui.text(format!("Moving: {label}"));
                                    src.end();
                                }

                                // --- Drop target: reparent onto this entity ---
                                if let Some(target) = ui.drag_drop_target() {
                                    if let Some(Ok(payload)) = target.accept_payload::<u64, _>(
                                        "entity_drag",
                                        imgui::DragDropFlags::empty(),
                                    ) {
                                        let raw = payload.data;
                                        let dragged = Entity::new(
                                            (raw & 0xFFFF_FFFF) as u32,
                                            (raw >> 32) as u32,
                                        );
                                        if dragged != *entity {
                                            reparent_req = Some((dragged, *entity));
                                        }
                                    }
                                }
                            }

                            // Drop onto empty area of the hierarchy = make root.
                            ui.dummy([0.0, ui.content_region_avail()[1].max(8.0)]);
                            if let Some(target) = ui.drag_drop_target() {
                                if let Some(Ok(payload)) = target.accept_payload::<u64, _>(
                                    "entity_drag",
                                    imgui::DragDropFlags::empty(),
                                ) {
                                    let raw = payload.data;
                                    let dragged =
                                        Entity::new((raw & 0xFFFF_FFFF) as u32, (raw >> 32) as u32);
                                    detach_req = Some(dragged);
                                }
                            }
                        }

                        if let Some(_tab) = ui.tab_item("Inspector") {
                            if let Some(entity) = selected {
                                ui.text(format!("{entity}"));
                                ui.separator();
                                ui.text("[ Tag ]");
                                if ui.input_text("tag##entity_tag", &mut new_tag).build() {
                                    tag_changed = true;
                                }
                                ui.separator();
                            } else {
                                ui.text_disabled("No entity selected.");
                                ui.separator();
                            }

                            // --- Transform (guaranteed, cannot be removed) ---
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
                                let tf_systems = systems_for_component(
                                    &self.state.system_component_map,
                                    "Transform",
                                );
                                if !tf_systems.is_empty() {
                                    ui.text_disabled(format!("  Used by: {tf_systems}"));
                                }
                                ui.separator();
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
                                let col_systems = systems_for_component(
                                    &self.state.system_component_map,
                                    "Color",
                                );
                                if !col_systems.is_empty() {
                                    ui.text_disabled(format!("  Used by: {col_systems}"));
                                }
                                if ui.small_button("Remove##rm_col") {
                                    remove_color = true;
                                }
                                ui.separator();
                            }

                            // --- Shape ---
                            if let Some(ref mut shape) = new_shape {
                                ui.text("[ Shape ]");
                                let shape_labels = ["Circle", "Rect", "Triangle"];
                                let mut shape_index = match shape {
                                    Shape::Circle { .. } => 0,
                                    Shape::Rect { .. } => 1,
                                    Shape::Triangle { .. } => 2,
                                };
                                if ui.combo_simple_string(
                                    "Type##shape",
                                    &mut shape_index,
                                    &shape_labels,
                                ) {
                                    *shape = match shape_index {
                                        0 => Shape::Circle { radius: 50.0 },
                                        1 => Shape::Rect {
                                            width: 100.0,
                                            height: 100.0,
                                        },
                                        _ => Shape::Triangle { size: 80.0 },
                                    };
                                    shape_changed = true;
                                }

                                match shape {
                                    Shape::Circle { radius } => {
                                        if ui.input_float("radius##shape", radius).build() {
                                            shape_changed = true;
                                        }
                                    }
                                    Shape::Rect { width, height } => {
                                        if ui.input_float("width##shape", width).build() {
                                            shape_changed = true;
                                        }
                                        if ui.input_float("height##shape", height).build() {
                                            shape_changed = true;
                                        }
                                    }
                                    Shape::Triangle { size } => {
                                        if ui.input_float("size##shape", size).build() {
                                            shape_changed = true;
                                        }
                                    }
                                }

                                let shape_systems = systems_for_component(
                                    &self.state.system_component_map,
                                    "Shape",
                                );
                                if !shape_systems.is_empty() {
                                    ui.text_disabled(format!("  Used by: {shape_systems}"));
                                }
                                if ui.small_button("Remove##rm_shape") {
                                    remove_shape = true;
                                }
                                ui.separator();
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
                                let sin_systems = systems_for_component(
                                    &self.state.system_component_map,
                                    "SinusoidComponent",
                                );
                                if !sin_systems.is_empty() {
                                    ui.text_disabled(format!("  Used by: {sin_systems}"));
                                }
                                if ui.small_button("Remove##rm_sin") {
                                    remove_sinusoid = true;
                                }
                                ui.separator();
                            }

                            // --- PlayerInput ---
                            if let Some(ref mut pi) = new_player_input {
                                ui.text("[ Player Input ]");
                                let entity =
                                    selected.expect("PlayerInput inspector requires selection");

                                ui.text("Horizontal axis:");
                                player_input_changed |= draw_player_input_binding_field(
                                    ui,
                                    &mut self.state,
                                    entity,
                                    pi,
                                    PlayerInputBinding::HorizontalNegative,
                                );
                                player_input_changed |= draw_player_input_binding_field(
                                    ui,
                                    &mut self.state,
                                    entity,
                                    pi,
                                    PlayerInputBinding::HorizontalPositive,
                                );

                                ui.text("Vertical axis:");
                                player_input_changed |= draw_player_input_binding_field(
                                    ui,
                                    &mut self.state,
                                    entity,
                                    pi,
                                    PlayerInputBinding::VerticalNegative,
                                );
                                player_input_changed |= draw_player_input_binding_field(
                                    ui,
                                    &mut self.state,
                                    entity,
                                    pi,
                                    PlayerInputBinding::VerticalPositive,
                                );

                                if ui.slider("Speed##pi", 0.0_f32, 1000.0, &mut pi.speed) {
                                    player_input_changed = true;
                                }

                                let pi_systems = systems_for_component(
                                    &self.state.system_component_map,
                                    "PlayerInput",
                                );
                                if !pi_systems.is_empty() {
                                    ui.text_disabled(format!("  Used by: {pi_systems}"));
                                }
                                if ui.small_button("Remove##rm_pi") {
                                    remove_player_input = true;
                                }
                                ui.separator();
                            }

                            // --- SpawnPoints ---
                            if let Some(ref mut sp) = new_spawn_points {
                                ui.text("[ Spawn Points ]");
                                ui.text(format!("{} slot(s)", sp.positions.len()));
                                let mut changed = false;
                                let mut remove_idx: Option<usize> = None;
                                for (i, pos) in sp.positions.iter_mut().enumerate() {
                                    let label_x = format!("x##{i}sp");
                                    let label_y = format!("y##{i}sp");
                                    let label_rm = format!("-##{i}sp");
                                    if ui.input_float(&label_x, &mut pos[0]).build() {
                                        changed = true;
                                    }
                                    ui.same_line();
                                    if ui.input_float(&label_y, &mut pos[1]).build() {
                                        changed = true;
                                    }
                                    ui.same_line();
                                    if ui.small_button(&label_rm) {
                                        remove_idx = Some(i);
                                    }
                                }
                                if let Some(i) = remove_idx {
                                    sp.positions.remove(i);
                                    changed = true;
                                }
                                if ui.small_button("+ Add Spawn Point") {
                                    sp.positions.push([0.0, 0.0, 0.0]);
                                    changed = true;
                                }
                                if changed {
                                    spawn_points_changed = true;
                                }
                                if ui.small_button("Remove##rm_sp") {
                                    remove_spawn_points = true;
                                }
                                ui.separator();
                            }

                            // --- Add Component dropdown ---
                            if let Some(entity) = selected {
                                let absent_indices: Vec<usize> = self
                                    .state
                                    .component_registry
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, d)| !(d.has)(&core.world, entity))
                                    .map(|(i, _)| i)
                                    .collect();

                                if !absent_indices.is_empty() {
                                    let absent_names: Vec<&str> = absent_indices
                                        .iter()
                                        .map(|&i| self.state.component_registry[i].name)
                                        .collect();

                                    if self.state.add_component_selection >= absent_indices.len() {
                                        self.state.add_component_selection = 0;
                                    }

                                    ui.combo_simple_string(
                                        "##add_comp",
                                        &mut self.state.add_component_selection,
                                        &absent_names,
                                    );
                                    ui.same_line();
                                    if ui.button("Add Component") {
                                        let registry_idx =
                                            absent_indices[self.state.add_component_selection];
                                        add_component_req = Some(registry_idx);
                                    }
                                }
                            }

                            ui.separator();
                            if ui.button("Despawn Entity") {
                                despawn_req = true;
                            }
                        }
                    }
                });

            // Scene controls only run when the pointer is over the scene itself.
            {
                let imgui_hovered = ui.is_window_hovered_with_flags(WindowHoveredFlags::ANY_WINDOW);
                let io = ui.io();
                let framebuffer_scale = [
                    io.display_framebuffer_scale[0].max(1.0),
                    io.display_framebuffer_scale[1].max(1.0),
                ];
                let mouse_pos = [
                    io.mouse_pos[0] * framebuffer_scale[0],
                    io.mouse_pos[1] * framebuffer_scale[1],
                ];
                let mouse_delta = [
                    io.mouse_delta[0] * framebuffer_scale[0],
                    io.mouse_delta[1] * framebuffer_scale[1],
                ];
                let mouse_in_viewport = (0.0..=viewport_w).contains(&mouse_pos[0])
                    && (0.0..=viewport_h).contains(&mouse_pos[1]);
                let scene_hovered = mouse_in_viewport && !imgui_hovered;
                let scene_input_available =
                    scene_hovered && !ui.is_any_item_active() && !io.want_capture_mouse;

                if self.state.scene_drag.is_some() && !ui.is_mouse_down(MouseButton::Left) {
                    self.state.scene_drag = None;
                }

                if scene_input_available && ui.is_mouse_double_clicked(MouseButton::Left) {
                    let world_pos = self.state.camera.screen_to_world(mouse_pos);
                    if let Some(entity) = pick_scene_entity(&core.world, world_pos) {
                        new_selected = Some(entity);
                    }
                }

                if scene_input_available && ui.is_mouse_clicked(MouseButton::Left) {
                    let world_pos = self.state.camera.screen_to_world(mouse_pos);
                    if let Some(entity) = pick_scene_entity(&core.world, world_pos) {
                        if let Some(tf) = core.world.get::<Transform>(entity) {
                            self.state.scene_drag = Some(SceneEntityDrag {
                                entity,
                                grab_offset: Vec2::new(
                                    world_pos.x - tf.position.x,
                                    world_pos.y - tf.position.y,
                                ),
                            });
                            new_selected = Some(entity);
                        }
                    }
                }

                if let Some(drag) = self.state.scene_drag {
                    if ui.is_mouse_down(MouseButton::Left) {
                        let world_pos = self.state.camera.screen_to_world(mouse_pos);
                        if let Some(tf) = core.world.get_mut::<Transform>(drag.entity) {
                            tf.position.x = world_pos.x - drag.grab_offset.x;
                            tf.position.y = world_pos.y - drag.grab_offset.y;
                            new_selected = Some(drag.entity);
                        } else {
                            self.state.scene_drag = None;
                        }
                    }
                }

                if scene_input_available
                    && (ui.is_mouse_down(MouseButton::Middle)
                        || ui.is_mouse_down(MouseButton::Right))
                {
                    cam_pan = mouse_delta;
                }

                let wheel = io.mouse_wheel;
                if scene_input_available && wheel != 0.0 {
                    cam_zoom = wheel;
                }
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

        // Hierarchy drag-to-reorder.
        if let Some((child, new_parent)) = reparent_req {
            // Guard against cycles: don't parent an ancestor onto its own descendant.
            let mut is_ancestor = false;
            let mut cursor = Some(new_parent);
            while let Some(cur) = cursor {
                if cur == child {
                    is_ancestor = true;
                    break;
                }
                cursor = core.world.scene_tree().parent(cur);
            }
            if !is_ancestor {
                core.world.scene_tree.attach(child, new_parent);
            }
        }
        if let Some(entity) = detach_req {
            core.world.scene_tree.detach(entity);
        }

        if let Some(entity) = selected {
            if tag_changed {
                if new_tag.trim().is_empty() {
                    core.world.remove::<Tag>(entity);
                } else {
                    core.world.insert(entity, Tag::new(new_tag));
                }
            }

            if transform_changed {
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

            if remove_shape {
                core.world.remove::<Shape>(entity);
            } else if shape_changed {
                if let Some(shape) = new_shape {
                    core.world.insert(entity, shape);
                }
            }

            if remove_sinusoid {
                core.world.remove::<SinusoidComponent>(entity);
            } else if sinusoid_changed {
                if let Some(s) = new_sinusoid {
                    core.world.insert(entity, s);
                }
            }

            if remove_player_input {
                core.world.remove::<PlayerInput>(entity);
            } else if player_input_changed {
                if let Some(pi) = new_player_input {
                    core.world.insert(entity, pi);
                }
            }

            if remove_spawn_points {
                core.world.remove::<SpawnPoints>(entity);
            } else if spawn_points_changed {
                if let Some(sp) = new_spawn_points {
                    core.world.insert(entity, sp);
                }
            }

            // Apply deferred "Add Component" from dropdown.
            if let Some(idx) = add_component_req {
                if let Some(desc) = self.state.component_registry.get(idx) {
                    (desc.add)(&mut core.world, entity);
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
            #[cfg(feature = "file-dialog")]
            {
                let current = std::path::Path::new(&self.state.scene_path);
                let mut dialog = rfd::FileDialog::new()
                    .add_filter("Scene JSON", &["json"])
                    .set_file_name(
                        current
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("scene.json"),
                    );
                if let Some(dir) = current.parent().filter(|d| d != &std::path::Path::new("")) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.save_file() {
                    let path_str = path.to_string_lossy().into_owned();
                    match save_scene(&core.world, &path_str) {
                        Ok(()) => {
                            self.state.status_message = format!("Saved → {path_str}");
                            self.state.scene_path = path_str;
                        }
                        Err(e) => self.state.status_message = format!("Save error: {e}"),
                    }
                }
            }

            #[cfg(not(feature = "file-dialog"))]
            {
                self.state.status_message = "Save dialog unavailable in this build".to_string();
            }
        }

        if load_req {
            #[cfg(feature = "file-dialog")]
            {
                let current = std::path::Path::new(&self.state.scene_path);
                let mut dialog = rfd::FileDialog::new().add_filter("Scene JSON", &["json"]);
                if let Some(dir) = current.parent().filter(|d| d != &std::path::Path::new("")) {
                    dialog = dialog.set_directory(dir);
                }
                if let Some(path) = dialog.pick_file() {
                    let path_str = path.to_string_lossy().into_owned();
                    match reload_scene(&mut core.world, &path_str) {
                        Ok(()) => {
                            self.state.status_message = format!("Loaded ← {path_str}");
                            self.state.scene_path = path_str;
                            self.state.selected_entity = None;
                            self.state.scene_drag = None;
                        }
                        Err(e) => self.state.status_message = format!("Load error: {e}"),
                    }
                }
            }

            #[cfg(not(feature = "file-dialog"))]
            {
                self.state.status_message = "Load dialog unavailable in this build".to_string();
            }
        }
    }
}

impl Default for EditorRunner {
    fn default() -> Self {
        Self::new()
    }
}

fn pick_scene_entity(world: &World, point: Vec2) -> Option<Entity> {
    world
        .query3::<Transform, Shape, Color>()
        .filter(|(_, transform, shape, _)| shape_contains_point(transform, shape, point))
        .max_by(|(a, a_tf, _, _), (b, b_tf, _, _)| {
            a_tf.position
                .z
                .partial_cmp(&b_tf.position.z)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.index.cmp(&b.index))
                .then_with(|| a.generation.cmp(&b.generation))
        })
        .map(|(entity, _, _, _)| entity)
}

fn shape_contains_point(transform: &Transform, shape: &Shape, point: Vec2) -> bool {
    let dx = point.x - transform.position.x;
    let dy = point.y - transform.position.y;

    match *shape {
        Shape::Circle { radius } => {
            let radius = radius.abs();
            dx * dx + dy * dy <= radius * radius
        }
        Shape::Rect { width, height } => {
            dx.abs() <= width.abs() * 0.5 && dy.abs() <= height.abs() * 0.5
        }
        Shape::Triangle { size } => {
            let size = size.abs();
            if size <= f32::EPSILON {
                return false;
            }

            let p = Vec2::new(dx / size, dy / size);
            let c = transform.rotation.cos();
            let s = transform.rotation.sin();
            let rotated = Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c);
            equilateral_triangle_sdf(rotated) <= 0.0
        }
    }
}

fn systems_for_component(entries: &[SystemComponentEntry], component: &str) -> String {
    let names: Vec<&str> = entries
        .iter()
        .filter(|entry| entry.component_names.contains(&component))
        .map(|entry| entry.system_name)
        .collect();
    names.join(", ")
}

fn equilateral_triangle_sdf(p: Vec2) -> f32 {
    let k = 3.0_f32.sqrt();
    let mut q = Vec2::new(p.x.abs() - 0.5, p.y + 0.5 / k);

    if q.x + k * q.y > 0.0 {
        q = Vec2::new(q.x - k * q.y, -k * q.x - q.y) * 0.5;
    }
    q.x -= q.x.clamp(-1.0, 0.0);

    -q.length() * q.y.signum()
}

fn player_input_binding_key_mut(
    input: &mut PlayerInput,
    binding: PlayerInputBinding,
) -> &mut ConfigKey {
    match binding {
        PlayerInputBinding::HorizontalNegative => &mut input.horizontal.negative,
        PlayerInputBinding::HorizontalPositive => &mut input.horizontal.positive,
        PlayerInputBinding::VerticalNegative => &mut input.vertical.negative,
        PlayerInputBinding::VerticalPositive => &mut input.vertical.positive,
    }
}

fn player_input_binding_id(binding: PlayerInputBinding) -> &'static str {
    match binding {
        PlayerInputBinding::HorizontalNegative => "h_neg",
        PlayerInputBinding::HorizontalPositive => "h_pos",
        PlayerInputBinding::VerticalNegative => "v_neg",
        PlayerInputBinding::VerticalPositive => "v_pos",
    }
}

fn draw_player_input_binding_field(
    ui: &imgui::Ui,
    state: &mut EditorState,
    entity: Entity,
    input: &mut PlayerInput,
    binding: PlayerInputBinding,
) -> bool {
    let key = *player_input_binding_key_mut(input, binding);
    let capture = PlayerInputCapture { entity, binding };
    let is_capturing = state.player_input_capture == Some(capture);
    let value = if is_capturing {
        "Press key..."
    } else {
        key.label()
    };
    let id = player_input_binding_id(binding);
    let button_label = format!("{}: {}##pi_bind_{id}", binding.label(), value);

    if ui.button(&button_label) {
        state.player_input_capture = Some(capture);
        state.status_message = format!("Press a key for {}", binding.label());
    }

    ui.same_line();
    let clear_label = format!("Clear##pi_bind_clear_{id}");
    if ui.small_button(&clear_label) {
        *player_input_binding_key_mut(input, binding) = ConfigKey::None;
        if is_capturing {
            state.player_input_capture = None;
        }
        return key != ConfigKey::None;
    }

    false
}

fn handle_player_input_capture(
    runner: &mut EditorRunner,
    core: &mut AppCore,
    event: &WindowEvent,
) -> bool {
    let Some(capture) = runner.state.player_input_capture else {
        return false;
    };
    let WindowEvent::KeyboardInput {
        event: key_event, ..
    } = event
    else {
        return false;
    };

    if key_event.state != ElementState::Pressed {
        return true;
    }

    let code = map_physical_key(key_event.physical_key);
    if code == KeyCode::Escape {
        runner.state.player_input_capture = None;
        runner.state.status_message = "PlayerInput binding cancelled".to_string();
        return true;
    }

    let Some(key) = config_key_from_key_code(code) else {
        runner.state.status_message = "Unsupported PlayerInput key".to_string();
        return true;
    };

    if let Some(input) = core.world.get_mut::<PlayerInput>(capture.entity) {
        *player_input_binding_key_mut(input, capture.binding) = key;
        runner.state.status_message =
            format!("Bound {} to {}", capture.binding.label(), key.label());
    } else {
        runner.state.status_message = "Selected entity no longer has PlayerInput".to_string();
    }
    runner.state.player_input_capture = None;
    true
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

        if handle_player_input_capture(&mut self.runner, core, &event) {
            return;
        }

        // Numpad double-input fix: when a numpad key produces a text character
        // (NumLock is ON), imgui-winit-support would map it to both a navigation
        // key (e.g. Numpad4 → LeftArrow) AND the character "4".  We skip the
        // full keyboard event and inject only the character manually so imgui
        // text fields receive the digit without unwanted cursor movement.
        let is_numpad_with_text = matches!(
            &event,
            WindowEvent::KeyboardInput { event: ke, .. }
                if ke.location == KeyLocation::Numpad
                    && ke.text.is_some()
                    && ke.state == ElementState::Pressed
        );

        if is_numpad_with_text {
            // Inject just the characters into imgui; skip key navigation.
            if let WindowEvent::KeyboardInput { event: ref ke, .. } = event {
                if let Some(ref text) = ke.text {
                    for ch in text.chars() {
                        core.imgui.add_input_character(ch);
                    }
                }
            }
        } else {
            core.imgui
                .handle_window_event(core.platform.window(), window_id, &event);
        }

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
