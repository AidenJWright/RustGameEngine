//! ImGui HUD for capture the flag.

use crate::ecs::world::World;
use crate::game::ctf::resources::{
    CarrierState, CtfHudState, CtfRestartState, GamePhase, GameState,
};
use crate::multiplayer::matchmaking::CtfSlot;
use crate::multiplayer::matchmaking::MapSize;

const HUD_WIDTH: f32 = 580.0;
const HUD_HEIGHT: f32 = 136.0;
const HUD_TOP: f32 = 12.0;

/// Draw black slot numbers centered over CTF player entities.
pub fn draw_player_numbers(ui: &imgui::Ui, positions: &[(CtfSlot, (f32, f32))]) {
    let draw_list = ui.get_foreground_draw_list();
    let scale = ui.io().display_framebuffer_scale;
    let scale_x = scale[0].max(f32::EPSILON);
    let scale_y = scale[1].max(f32::EPSILON);
    for (slot, (x, y)) in positions {
        let text = slot.team_number().to_string();
        let size = ui.calc_text_size(&text);
        let logical_x = x / scale_x;
        let logical_y = y / scale_y;
        draw_list.add_text(
            [logical_x - size[0] * 0.5, logical_y - size[1] * 0.5],
            imgui::ImColor32::BLACK,
            text,
        );
    }
}

/// Draw the CTF status HUD and win overlay.
pub fn draw_hud(ui: &imgui::Ui, world: &mut World) {
    let display = ui.io().display_size;
    let carrier = world
        .resource::<CarrierState>()
        .copied()
        .unwrap_or_default();
    let phase = world
        .resource::<GameState>()
        .map_or(GamePhase::Playing, |state| state.phase);
    let current_map_size = world.resource::<MapSize>().copied().unwrap_or_default();
    let restart_state = world
        .resource::<CtfRestartState>()
        .copied()
        .unwrap_or(CtfRestartState {
            selected_map_size: current_map_size,
            pending_reload: None,
        });
    let mut hud_state = world.resource::<CtfHudState>().copied().unwrap_or_default();

    if hud_state.collapsed {
        ui.window("CTF HUD Toggle")
            .position([display[0] * 0.5 - 42.0, HUD_TOP], imgui::Condition::Always)
            .bg_alpha(0.78)
            .flags(
                imgui::WindowFlags::NO_TITLE_BAR
                    | imgui::WindowFlags::NO_RESIZE
                    | imgui::WindowFlags::NO_MOVE
                    | imgui::WindowFlags::NO_COLLAPSE
                    | imgui::WindowFlags::NO_SAVED_SETTINGS
                    | imgui::WindowFlags::ALWAYS_AUTO_RESIZE,
            )
            .build(|| {
                if ui.arrow_button("show_ctf_hud", imgui::Direction::Down) {
                    hud_state.collapsed = false;
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text("Show CTF HUD");
                }
                ui.same_line();
                ui.text("CTF");
            });
        world.insert_resource(hud_state);
    } else {
        ui.window("CTF")
            .position(
                [display[0] * 0.5 - HUD_WIDTH * 0.5, HUD_TOP],
                imgui::Condition::Always,
            )
            .size([HUD_WIDTH, HUD_HEIGHT], imgui::Condition::Always)
            .bg_alpha(0.78)
            .flags(
                imgui::WindowFlags::NO_RESIZE
                    | imgui::WindowFlags::NO_MOVE
                    | imgui::WindowFlags::NO_COLLAPSE
                    | imgui::WindowFlags::NO_SAVED_SETTINGS,
            )
            .build(|| {
                if ui.arrow_button("hide_ctf_hud", imgui::Direction::Up) {
                    hud_state.collapsed = true;
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text("Hide CTF HUD");
                }
                ui.same_line();
                ui.text("Capture the Flag");
                ui.separator();
                ui.text(if let Some(slot) = carrier.blue_flag_carrier {
                    format!("Red: {} has blue flag", slot.label())
                } else {
                    "Red: blue flag needed".to_string()
                });
                ui.text(if let Some(slot) = carrier.red_flag_carrier {
                    format!("Blue: {} has red flag", slot.label())
                } else {
                    "Blue: red flag needed".to_string()
                });
                ui.text_disabled("Move: WASD/Arrows  Throw: Space  Switch: Left Shift/1-4");
                ui.text_disabled("Click: auto-move  Restart after win: R  Cam-follow: Left Ctrl");
            });
        world.insert_resource(hud_state);
    }

    if let GamePhase::Won(id) = phase {
        ui.window("CTF Victory")
            .position(
                [display[0] * 0.5 - 180.0, display[1] * 0.5 - 72.0],
                imgui::Condition::Always,
            )
            .size([360.0, 174.0], imgui::Condition::Always)
            .bg_alpha(0.9)
            .flags(
                imgui::WindowFlags::NO_RESIZE
                    | imgui::WindowFlags::NO_MOVE
                    | imgui::WindowFlags::NO_COLLAPSE
                    | imgui::WindowFlags::NO_SAVED_SETTINGS,
            )
            .build(|| {
                let winner = if id == 1 {
                    "Red Team Wins!"
                } else {
                    "Blue Team Wins!"
                };
                ui.text(winner);
                ui.separator();
                let mut selected_idx = MapSize::ALL
                    .iter()
                    .position(|map_size| *map_size == restart_state.selected_map_size)
                    .unwrap_or(0);
                let labels = MapSize::ALL.map(MapSize::label);
                if ui.combo_simple_string("Level Size", &mut selected_idx, &labels) {
                    world.insert_resource(CtfRestartState {
                        selected_map_size: MapSize::ALL[selected_idx],
                        pending_reload: restart_state.pending_reload,
                    });
                }
                ui.text("Press R to restart.");
            });
    }
}
