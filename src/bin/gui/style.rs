use eframe::egui;

use diycpu::types::{is_primitive, AbstractionDef, ComponentDef};

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
pub const PORT_SPACING: f32 = 20.0;
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 3.0;
pub const WAYPOINT_HANDLE_SIZE: f32 = 6.0;
pub const WAYPOINT_COLOR: egui::Color32 = egui::Color32::from_rgb(180, 180, 255);
pub const WAYPOINT_HOVER_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 255, 100);
pub const INPUT_PIN_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 220, 255);
pub const OUTPUT_PIN_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 160, 100);

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
/// Used for primitives with a single output.
pub fn output_port_position(component_rect: egui::Rect) -> egui::Pos2 {
    egui::pos2(
        component_rect.max.x,
        component_rect.min.y + component_rect.height() / 2.0,
    )
}

/// Returns (screen_position, output_name) for each output port of a component.
/// Primitives get a single "out" port. Abstraction instances get one port per
/// declared output, evenly distributed along the right edge.
pub fn output_port_positions(
    component_rect: egui::Rect,
    component: &ComponentDef,
    abstractions: &[AbstractionDef],
) -> Vec<(egui::Pos2, String)> {
    if is_primitive(&component.component_type) {
        return vec![(output_port_position(component_rect), "out".into())];
    }

    if let Some(abstraction) = abstractions.iter().find(|a| a.name == component.component_type) {
        let mut output_names: Vec<&String> = abstraction.outputs.keys().collect();
        output_names.sort();
        let count = output_names.len().max(1) as f32;
        return output_names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let y = component_rect.min.y
                    + component_rect.height() * (i as f32 + 1.0) / (count + 1.0);
                (egui::pos2(component_rect.max.x, y), (*name).clone())
            })
            .collect();
    }

    // Unknown type fallback: single "out" port
    vec![(output_port_position(component_rect), "out".into())]
}

/// Compute the minimum distance from a point to a line segment.
pub fn point_to_segment_distance(
    point: egui::Pos2,
    seg_start: egui::Pos2,
    seg_end: egui::Pos2,
) -> f32 {
    let dx = seg_end.x - seg_start.x;
    let dy = seg_end.y - seg_start.y;
    let length_squared = dx * dx + dy * dy;

    if length_squared < 1e-6 {
        return point.distance(seg_start);
    }

    let t = ((point.x - seg_start.x) * dx + (point.y - seg_start.y) * dy) / length_squared;
    let t = t.clamp(0.0, 1.0);
    let closest = egui::pos2(seg_start.x + t * dx, seg_start.y + t * dy);
    point.distance(closest)
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
