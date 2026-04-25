//! ImGui HUD for capture the flag.

use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, CtfRestartState, GamePhase, GameState};
use crate::multiplayer::matchmaking::MapSize;

/// Draw the CTF status HUD and win overlay.
pub fn draw_hud(ui: &imgui::Ui, world: &mut World) {
    let display = ui.io().display_size;
    let carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
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

    ui.window("CTF")
        .position([display[0] * 0.5 - 170.0, 12.0], imgui::Condition::Always)
        .size([340.0, 112.0], imgui::Condition::Always)
        .bg_alpha(0.78)
        .flags(
            imgui::WindowFlags::NO_RESIZE
                | imgui::WindowFlags::NO_MOVE
                | imgui::WindowFlags::NO_COLLAPSE
                | imgui::WindowFlags::NO_SAVED_SETTINGS,
        )
        .build(|| {
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
            ui.text_disabled("Move: WASD/Arrows  Primary: Space/Return  Switch: Left Shift");
            ui.text_disabled("Click: auto-move  Restart after win: R  Cam-follow: Left Ctrl");
        });

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
                let winner = if id == 1 { "Red Team Wins!" } else { "Blue Team Wins!" };
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
