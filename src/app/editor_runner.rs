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
    Color, PlayerInput, Shape, SinusoidComponent, SpawnPoints, Tag, Transform,
};
use crate::ecs::entity::Entity;
use crate::ecs::resource::{DeltaTime, ElapsedTime};
use crate::ecs::world::World;
use crate::editor::EditorState;
use crate::messaging::MessageBus;
use crate::renderer::draw::DrawCommand;
use crate::scene::{load_scene, save_scene};

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

        // Viewport is the area left of the inspector panel and below the toolbar.
        // Shapes are rendered into this region, not the full surface.
        let toolbar_h = 30.0_f32;
        let status_h  = 22.0_f32;
        let viewport_w = (surface_w - self.state.inspector_width).max(1.0);
        let viewport_h = (surface_h - toolbar_h - status_h).max(1.0);

        // --- 1. Build scene draw-commands with camera transform ---
        let draw_cmds: Vec<DrawCommand> = core
            .world
            .query3::<Transform, Shape, Color>()
            .map(|(_, t, s, c)| {
                let raw = make_draw_cmd(t, s, c); // world-origin coords
                self.state.camera.transform_draw_cmd(raw, viewport_w, viewport_h)
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
        let (mut new_transform, mut new_color, mut new_sinusoid, mut new_player_input, mut new_spawn_points) =
            if let Some(e) = selected {
                (
                    core.world.get::<Transform>(e).cloned(),
                    core.world.get::<Color>(e).cloned(),
                    core.world.get::<SinusoidComponent>(e).cloned(),
                    core.world.get::<PlayerInput>(e).cloned(),
                    core.world.get::<SpawnPoints>(e).cloned(),
                )
            } else {
                (None, None, None, None, None)
            };

        // --- 4. Action flags collected during UI ---
        let mut new_selected = selected;
        let mut transform_changed = false;
        let mut color_changed = false;
        let mut sinusoid_changed = false;
        let mut player_input_changed = false;
        let mut spawn_points_changed = false;
        let mut remove_color = false;
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
        // Inspector width read back from imgui each frame.
        let mut captured_inspector_width = self.state.inspector_width;

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

            // Main dockspace (enables real docking behavior for panels).
            //
            // imgui-rs doesn't expose docking as safe wrappers here, so we use
            // the low-level imgui::sys API.
            let dockspace_id = ui.new_id_str("##main_dockspace");
            let dockspace_u32: u32 = unsafe { std::mem::transmute::<imgui::Id, u32>(dockspace_id) };
            let _ = dockspace_u32; // used only for igDockSpace call below

            // Use ImGui's logical display size for layout/docking math (important on HiDPI).
            let display_w = ui.io().display_size[0];
            let display_h = ui.io().display_size[1];

            // Keep dockspace below toolbar and above status bar.
            let toolbar_h = 30.0_f32;
            let status_h = 22.0_f32;
            let dock_pos = [0.0, toolbar_h];
            let dock_size = [display_w, (display_h - toolbar_h - status_h).max(1.0)];

            ui.window("##dockspace_host")
                .no_decoration()
                .draw_background(false)
                .position(dock_pos, imgui::Condition::Always)
                .size(dock_size, imgui::Condition::Always)
                .flags(
                    imgui::WindowFlags::NO_SAVED_SETTINGS
                        | imgui::WindowFlags::NO_MOVE
                        | imgui::WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
                        | imgui::WindowFlags::NO_NAV_FOCUS,
                )
                .build(|| unsafe {
                    // Create the dockspace — panels can be freely dragged and
                    // docked anywhere inside it.
                    imgui::sys::igDockSpace(
                        dockspace_u32,
                        imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
                        0,
                        std::ptr::null(),
                    );
                });

            // Toolbar
            ui.window("##toolbar")
                .no_decoration()
                .size([display_w, toolbar_h], imgui::Condition::Always)
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

            // Hierarchy panel — drag-and-drop to reorder / reparent.
            ui.window("Scene Hierarchy")
                .position([0.0, toolbar_h], imgui::Condition::FirstUseEver)
                .size([180.0, dock_size[1]], imgui::Condition::FirstUseEver)
                .build(|| {
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
                            let dragged = Entity::new(
                                (raw & 0xFFFF_FFFF) as u32,
                                (raw >> 32) as u32,
                            );
                            detach_req = Some(dragged);
                        }
                    }
                });

            // Inspector panel.
            ui.window("Inspector")
                .position([display_w - self.state.inspector_width, toolbar_h], imgui::Condition::FirstUseEver)
                .size(
                    [self.state.inspector_width, (display_h - toolbar_h - status_h).max(1.0)],
                    imgui::Condition::FirstUseEver,
                )
                .flags(imgui::WindowFlags::NO_SAVED_SETTINGS)
                .build(|| {
                    // Read back actual width so we can update the anchor next frame.
                    captured_inspector_width = ui.window_size()[0];

                    if let Some(entity) = selected {
                        ui.text(format!("{entity}"));
                        ui.separator();
                    } else {
                        ui.text_disabled("No entity selected.");
                        ui.separator();
                    }

                        // Helper: collect system names that read a given component.
                        let systems_for = |comp: &str| -> String {
                            let names: Vec<&str> = self
                                .state
                                .system_component_map
                                .iter()
                                .filter(|e| e.component_names.contains(&comp))
                                .map(|e| e.system_name)
                                .collect();
                            names.join(", ")
                        };

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
                            let tf_systems = systems_for("Transform");
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
                            let col_systems = systems_for("Color");
                            if !col_systems.is_empty() {
                                ui.text_disabled(format!("  Used by: {col_systems}"));
                            }
                            if ui.small_button("Remove##rm_col") {
                                remove_color = true;
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
                            let sin_systems = systems_for("SinusoidComponent");
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

                            // Build label lists for combo boxes.
                            use crate::components::ConfigKey;
                            let key_labels: Vec<&str> =
                                ConfigKey::ALL.iter().map(|k| k.label()).collect();

                            let mut h_neg = pi.horizontal.negative.index();
                            let mut h_pos = pi.horizontal.positive.index();
                            let mut v_neg = pi.vertical.negative.index();
                            let mut v_pos = pi.vertical.positive.index();

                            ui.text("Horizontal axis:");
                            if ui.combo_simple_string("H-##pi", &mut h_neg, &key_labels) {
                                pi.horizontal.negative = ConfigKey::ALL[h_neg];
                                player_input_changed = true;
                            }
                            ui.same_line();
                            ui.text("neg");
                            if ui.combo_simple_string("H+##pi", &mut h_pos, &key_labels) {
                                pi.horizontal.positive = ConfigKey::ALL[h_pos];
                                player_input_changed = true;
                            }
                            ui.same_line();
                            ui.text("pos");

                            ui.text("Vertical axis:");
                            if ui.combo_simple_string("V-##pi", &mut v_neg, &key_labels) {
                                pi.vertical.negative = ConfigKey::ALL[v_neg];
                                player_input_changed = true;
                            }
                            ui.same_line();
                            ui.text("neg");
                            if ui.combo_simple_string("V+##pi", &mut v_pos, &key_labels) {
                                pi.vertical.positive = ConfigKey::ALL[v_pos];
                                player_input_changed = true;
                            }
                            ui.same_line();
                            ui.text("pos");

                            if ui.slider("Speed##pi", 0.0_f32, 1000.0, &mut pi.speed) {
                                player_input_changed = true;
                            }

                            let pi_systems = systems_for("PlayerInput");
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
                });

            // Camera control via imgui IO (middle-drag to pan, scroll to zoom)
            {
                let io = ui.io();
                if (io.mouse_down[2] || io.mouse_down[1]) && !ui.is_any_item_active() {
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
                    .size([display_w, status_h], imgui::Condition::Always)
                    .position([0.0, display_h - status_h], imgui::Condition::Always)
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
        self.state.inspector_width = captured_inspector_width;
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