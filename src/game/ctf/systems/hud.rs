//! ImGui HUD for capture the flag.

use crate::ecs::world::World;
use crate::game::ctf::resources::{CarrierState, GamePhase, GameState};

/// Draw the CTF status HUD and win overlay.
pub fn draw_hud(ui: &imgui::Ui, world: &World) {
    let display = ui.io().display_size;
    let carrier = world.resource::<CarrierState>().copied().unwrap_or_default();
    let phase = world
        .resource::<GameState>()
        .map_or(GamePhase::Playing, |state| state.phase);

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
            ui.text_disabled("Click: auto-move  Restart: R  Cam-follow: Left Ctrl");
        });

    if let GamePhase::Won(id) = phase {
        ui.window("CTF Victory")
            .position(
                [display[0] * 0.5 - 180.0, display[1] * 0.5 - 72.0],
                imgui::Condition::Always,
            )
            .size([360.0, 144.0], imgui::Condition::Always)
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
                ui.text("Press R to restart.");
            });
    }
}
