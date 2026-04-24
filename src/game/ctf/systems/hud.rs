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
            ui.text(if carrier.p1_carries {
                "P1: has blue flag"
            } else {
                "P1: flag safe"
            });
            ui.text(if carrier.p2_carries {
                "P2: has red flag"
            } else {
                "P2: flag safe"
            });
            ui.text_disabled("P1: WASD + Space    P2: Arrows + Return");
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
                ui.text(format!("Player {id} Wins!"));
                ui.separator();
                ui.text("Close and run again to restart.");
            });
    }
}
