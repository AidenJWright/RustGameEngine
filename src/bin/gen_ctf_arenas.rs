//! Procedural CTF arena generator.
//!
//! Run `cargo run --bin gen_ctf_arenas` to regenerate the medium and large
//! arena JSON files.  The outputs are static assets committed to the repo;
//! do NOT regenerate them at game runtime.
//!
//! Arena sizes:
//!   ctf_arena_small  — 1280 × 720   (1×, hand-authored)
//!   ctf_arena_medium — 2560 × 1440  (4× area, generated)
//!   ctf_arena_large  — 3840 × 2160  (9× area, generated)

use serde_json::{json, Value};

fn main() {
    write_arena("assets/ctf_arena_medium.json", 2, 0.5);
    write_arena("assets/ctf_arena_large.json", 3, 1.0 / 3.0);
    println!("done.");
}

fn write_arena(path: &str, scale: u32, zoom: f64) {
    let v = build_arena(scale, zoom);
    let text = serde_json::to_string_pretty(&v).expect("JSON serialization failed");
    std::fs::write(path, text)
        .unwrap_or_else(|e| panic!("failed to write {path}: {e}"));
    println!("wrote {path}");
}

// ---------------------------------------------------------------------------
// Arena builder
// ---------------------------------------------------------------------------

fn build_arena(scale: u32, zoom: f64) -> Value {
    let s = scale as f64;
    let w = 1280.0 * s;
    let h = 720.0 * s;
    let mid_x = w / 2.0;
    let mid_y = h / 2.0;

    let mut id: u64 = 0;
    let mut ents: Vec<Value> = Vec::new();

    // scene root (no parent, no transform)
    ents.push(json!({ "id": id, "tag": "scene_root" }));
    id += 1;

    // ── background zones ────────────────────────────────────────────────────
    ents.push(bg_rect(id, "p1_zone",  mid_x / 2.0,  mid_y, -10.0, mid_x,       h, [1.0, 0.2, 0.2, 0.12]));
    id += 1;
    ents.push(bg_rect(id, "p2_zone",  mid_x * 1.5,  mid_y, -10.0, mid_x,       h, [0.2, 0.4, 1.0, 0.12]));
    id += 1;
    ents.push(bg_rect(id, "divider",  mid_x,         mid_y,  -5.0, 4.0 * s, h, [0.7, 0.7, 0.7, 0.5]));
    id += 1;

    // ── flag-base walls (U-pockets, same proportions as small, scaled) ───────
    let back_w  = 20.0 * s;
    let back_h  = 220.0 * s;
    let arm_w   = 190.0 * s;
    let arm_h   = 20.0 * s;
    let back_cx = 90.0 * s;
    let arm_cx  = 175.0 * s;
    let arm_top = 250.0 * s;
    let arm_bot = 470.0 * s;

    // red base
    ents.push(wall(id, back_cx,       mid_y,   0.0, back_w, back_h, BASE_COL));  id += 1;
    ents.push(wall(id, arm_cx,        arm_top, 0.0, arm_w,  arm_h,  BASE_COL));  id += 1;
    ents.push(wall(id, arm_cx,        arm_bot, 0.0, arm_w,  arm_h,  BASE_COL));  id += 1;
    // blue base (mirror)
    ents.push(wall(id, w - back_cx,   mid_y,   0.0, back_w, back_h, BASE_COL));  id += 1;
    ents.push(wall(id, w - arm_cx,    arm_top, 0.0, arm_w,  arm_h,  BASE_COL));  id += 1;
    ents.push(wall(id, w - arm_cx,    arm_bot, 0.0, arm_w,  arm_h,  BASE_COL));  id += 1;

    // ── midfield obstacles ──────────────────────────────────────────────────
    for (cx, cy, ww, hh) in midfield_specs(scale, w, h) {
        ents.push(wall(id, cx, cy, 0.0, ww, hh, MID_COL));
        id += 1;
    }

    // ── boundary walls ──────────────────────────────────────────────────────
    let border_off   = 5.0 * s;
    let border_thick = 10.0 * s;
    ents.push(wall(id, mid_x,             border_off,      1.0, w,            border_thick, EDGE_COL));  id += 1;
    ents.push(wall(id, mid_x,             h - border_off,  1.0, w,            border_thick, EDGE_COL));  id += 1;
    ents.push(wall(id, border_off,        mid_y,           1.0, border_thick, h,            EDGE_COL));  id += 1;
    ents.push(wall(id, w - border_off,    mid_y,           1.0, border_thick, h,            EDGE_COL));  id += 1;

    // ── flags ───────────────────────────────────────────────────────────────
    let flag_x = 160.0 * s;
    ents.push(json!({
        "id": id, "parent": 0, "tag": "ctf_flag_red",
        "transform": xform(flag_x, mid_y, 5.0),
        "shape": { "Circle": { "radius": 12.0 } },
        "color": { "r": 1.0, "g": 0.05, "b": 0.05, "a": 1.0 }
    }));
    id += 1;
    ents.push(json!({
        "id": id, "parent": 0, "tag": "ctf_flag_blue",
        "transform": xform(w - flag_x, mid_y, 5.0),
        "shape": { "Circle": { "radius": 12.0 } },
        "color": { "r": 0.05, "g": 0.22, "b": 1.0, "a": 1.0 }
    }));
    id += 1;

    // ── players ─────────────────────────────────────────────────────────────
    let spawn_offsets = [-60.0 * s, -20.0 * s, 20.0 * s, 60.0 * s];
    let red_colors = [
        [1.0, 0.18, 0.18, 1.0],
        [1.0, 0.28, 0.28, 1.0],
        [0.9, 0.12, 0.12, 1.0],
        [1.0, 0.38, 0.38, 1.0],
    ];
    let blue_colors = [
        [0.18, 0.44, 1.0, 1.0],
        [0.28, 0.52, 1.0, 1.0],
        [0.1, 0.34, 0.9, 1.0],
        [0.38, 0.62, 1.0, 1.0],
    ];
    for (idx, offset) in spawn_offsets.iter().enumerate() {
        ents.push(player(
            id,
            &format!("ctf_player_red_{}", idx + 1),
            flag_x,
            mid_y + offset,
            PI_2,
            red_colors[idx],
        ));
        id += 1;
    }
    for (idx, offset) in spawn_offsets.iter().enumerate() {
        ents.push(player(
            id,
            &format!("ctf_player_blue_{}", idx + 1),
            w - flag_x,
            mid_y + offset,
            -PI_2,
            blue_colors[idx],
        ));
        id += 1;
    }

    // ── camera ──────────────────────────────────────────────────────────────
    ents.push(json!({
        "id": id, "parent": 0, "tag": "ctf_camera",
        "transform": xform(mid_x, mid_y, 0.0),
        "camera": { "position": { "x": 0.0, "y": 0.0 }, "zoom": zoom }
    }));

    json!({ "version": 1, "entities": ents })
}

// ---------------------------------------------------------------------------
// Midfield obstacle layout
//
// Obstacles are arranged to create three routing lanes (top / middle / bottom)
// that alternate which lane is blocked at each column, forcing diagonal paths.
//
// Each scale level adds columns proportional to the extra arena space:
//   scale 2 (4× area): 4 obstacles per team side + 4 center obstacles
//   scale 3 (9× area): 6 obstacles per team side + 6 center obstacles
//
// Red-side coordinates are specified; blue-side is mirrored around mid_x.
// ---------------------------------------------------------------------------

fn midfield_specs(scale: u32, w: f64, _h: f64) -> Vec<(f64, f64, f64, f64)> {
    let mid_x = w / 2.0;
    let mut specs: Vec<(f64, f64, f64, f64)> = Vec::new();

    match scale {
        2 => {
            // ── red side ────────────────────────────────────────────────────
            // Two scaled-up originals (original × 2)
            let red = [
                (760.0,  360.0,  40.0, 260.0),  // scaled from small obs A (vertical, upper)
                (900.0, 1120.0, 260.0,  40.0),  // scaled from small obs B (horizontal, lower)
                // Two new obstacles filling the extra vertical space
                (640.0,  980.0,  40.0, 260.0),  // lower vertical near base
                (1100.0, 380.0, 260.0,  40.0),  // upper horizontal near center
            ];
            for spec in red {
                specs.push(spec);
                specs.push(mirror_x(spec, mid_x)); // blue-side mirror
            }

            // ── center (not mirrored, already at mid_x) ──────────────────
            specs.push((1280.0,  240.0, 120.0, 160.0)); // center top
            specs.push((1280.0, 1200.0, 120.0, 160.0)); // center bottom
            // small flanking blockers that create a diamond gap at mid-line
            specs.push((1180.0,  660.0,  40.0, 160.0)); // center-left
            specs.push((1380.0,  780.0,  40.0, 160.0)); // center-right
        }

        3 => {
            // ── red side ────────────────────────────────────────────────────
            // Two scaled-up originals (original × 3) plus four new obstacles
            // distributed across the wider mid-field space
            let red = [
                (1140.0,  540.0,  60.0, 390.0),  // scaled A: vertical, upper area
                (1350.0, 1680.0, 390.0,  60.0),  // scaled B: horizontal, lower area
                (960.0,  1470.0,  60.0, 390.0),  // new C: lower vertical near base
                (1650.0,  570.0, 390.0,  60.0),  // new D: upper horizontal near center
                (870.0,   480.0,  60.0, 390.0),  // new E: upper vertical near base
                (1560.0, 1590.0, 390.0,  60.0),  // new F: lower horizontal near center
            ];
            for spec in red {
                specs.push(spec);
                specs.push(mirror_x(spec, mid_x));
            }

            // ── center ───────────────────────────────────────────────────
            specs.push((1920.0,  360.0, 180.0, 240.0)); // center top
            specs.push((1920.0, 1800.0, 180.0, 240.0)); // center bottom
            // diamond pattern at center line
            specs.push((1820.0,  900.0,  60.0, 240.0)); // upper-left
            specs.push((2020.0, 1260.0,  60.0, 240.0)); // lower-right
            specs.push((1820.0, 1260.0,  60.0, 240.0)); // lower-left
            specs.push((2020.0,  900.0,  60.0, 240.0)); // upper-right
        }

        _ => panic!("unsupported scale {scale}; only 2 and 3 are implemented"),
    }

    specs
}

/// Reflect a (cx, cy, w, h) obstacle spec through x = mid_x.
fn mirror_x(spec: (f64, f64, f64, f64), mid_x: f64) -> (f64, f64, f64, f64) {
    (mid_x * 2.0 - spec.0, spec.1, spec.2, spec.3)
}

// ---------------------------------------------------------------------------
// JSON entity helpers
// ---------------------------------------------------------------------------

const BASE_COL: [f64; 4] = [0.75, 0.78, 0.82, 1.0];
const MID_COL:  [f64; 4] = [0.55, 0.58, 0.64, 1.0];
const EDGE_COL: [f64; 4] = [0.35, 0.38, 0.42, 1.0];
const PI_2: f64 = std::f64::consts::FRAC_PI_2;

fn xform(x: f64, y: f64, z: f64) -> Value {
    json!({
        "position": { "x": x, "y": y, "z": z },
        "rotation": 0.0,
        "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
    })
}

fn bg_rect(id: u64, tag: &str, x: f64, y: f64, z: f64, w: f64, h: f64, c: [f64; 4]) -> Value {
    json!({
        "id": id, "parent": 0, "tag": tag,
        "transform": xform(x, y, z),
        "shape": { "Rect": { "width": w, "height": h } },
        "color": { "r": c[0], "g": c[1], "b": c[2], "a": c[3] }
    })
}

fn wall(id: u64, x: f64, y: f64, z: f64, w: f64, h: f64, c: [f64; 4]) -> Value {
    json!({
        "id": id, "parent": 0, "tag": "wall",
        "transform": xform(x, y, z),
        "shape": { "Rect": { "width": w, "height": h } },
        "color": { "r": c[0], "g": c[1], "b": c[2], "a": c[3] }
    })
}

fn player(id: u64, tag: &str, x: f64, y: f64, rot: f64, c: [f64; 4]) -> Value {
    json!({
        "id": id, "parent": 0, "tag": tag,
        "transform": {
            "position": { "x": x, "y": y, "z": 10.0 },
            "rotation": rot,
            "scale": { "x": 1.0, "y": 1.0, "z": 1.0 }
        },
        "shape": { "Triangle": { "size": 40.0 } },
        "color": { "r": c[0], "g": c[1], "b": c[2], "a": c[3] },
        "velocity": { "dx": 0.0, "dy": 0.0 }
    })
}
