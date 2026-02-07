use eframe::egui;

use diycpu::types::ComponentDef;

// ── Component colors ─────────────────────────────────────────────────────────

pub const CLOCK_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 180, 255);
pub const NAND_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 80);
pub const DFF_COLOR: egui::Color32 = egui::Color32::from_rgb(130, 220, 130);
pub const ABSTRACTION_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 160, 255);

// ── Wire colors ──────────────────────────────────────────────────────────────

pub const WIRE_HIGH: egui::Color32 = egui::Color32::from_rgb(80, 255, 80);
pub const WIRE_LOW: egui::Color32 = egui::Color32::from_rgb(80, 80, 80);
pub const WIRE_UNKNOWN: egui::Color32 = egui::Color32::from_gray(140);
pub const PORT_HOVER_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 255, 100);

// ── Canvas dimensions ────────────────────────────────────────────────────────

pub const COMPONENT_WIDTH: f32 = 140.0;
pub const COMPONENT_HEIGHT: f32 = 70.0;
pub const GRID_SPACING: f32 = 20.0;
pub const PORT_RADIUS: f32 = 5.0;
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 3.0;

// ── Helper functions ─────────────────────────────────────────────────────────

pub fn color_for_component_type(component_type: &str) -> egui::Color32 {
    match component_type {
        "clk" => CLOCK_COLOR,
        "nand" => NAND_COLOR,
        "dff" => DFF_COLOR,
        _ => ABSTRACTION_COLOR,
    }
}

pub fn snap_to_grid(value: f32) -> f32 {
    (value / GRID_SPACING).round() * GRID_SPACING
}

// ── Port geometry ────────────────────────────────────────────────────────────

/// Returns (screen_position, port_label) for each input port of a component.
/// Input ports are evenly distributed along the left edge of the component.
pub fn input_port_positions(
    component_rect: egui::Rect,
    component: &ComponentDef,
) -> Vec<(egui::Pos2, String)> {
    let sorted_keys = sorted_input_keys(component);
    let port_count = sorted_keys.len() as f32;
    sorted_keys
        .iter()
        .enumerate()
        .map(|(port_index, port_name)| {
            let vertical_position = component_rect.min.y
                + component_rect.height() * (port_index as f32 + 1.0) / (port_count + 1.0);
            (
                egui::pos2(component_rect.min.x, vertical_position),
                port_name.clone(),
            )
        })
        .collect()
}

/// Returns the output port position (right edge, vertically centered).
pub fn output_port_position(component_rect: egui::Rect) -> egui::Pos2 {
    egui::pos2(
        component_rect.max.x,
        component_rect.min.y + component_rect.height() / 2.0,
    )
}

/// Returns the sorted input keys for a component (for deterministic port ordering).
pub fn sorted_input_keys(component: &ComponentDef) -> Vec<String> {
    if let Some(ref inputs) = component.inputs {
        let mut keys: Vec<String> = inputs.keys().cloned().collect();
        keys.sort();
        keys
    } else {
        vec![]
    }
}
