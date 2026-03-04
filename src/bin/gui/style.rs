use eframe::egui;

use diycpu::project::{Endpoint, ProjectAbstraction, WireNet};
use diycpu::types::is_primitive;

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
pub const INPUT_PIN_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 220, 255);
pub const OUTPUT_PIN_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 160, 100);

// ── Wire / junction constants ────────────────────────────────────────────────

pub const WIRE_THICKNESS: f32 = 4.0;
pub const WIRE_OUTLINE: f32 = 1.5;
pub const JUNCTION_RADIUS: f32 = 6.0;

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

// ── Port name helpers ────────────────────────────────────────────────────────

/// Returns the sorted input port names for a component type.
pub fn input_port_names(
    component_type: &str,
    abstractions: &[ProjectAbstraction],
) -> Vec<String> {
    let mut names = match component_type {
        "clk" => vec![],
        "nand" => vec!["a".into(), "b".into()],
        "dff" => vec!["clk".into(), "d".into()],
        _ => {
            if let Some(abs) = abstractions.iter().find(|a| a.name == component_type) {
                abs.inputs.clone()
            } else {
                vec![]
            }
        }
    };
    names.sort();
    names
}

/// Returns the sorted output port names for a component type.
pub fn output_port_names(
    component_type: &str,
    abstractions: &[ProjectAbstraction],
) -> Vec<String> {
    if is_primitive(component_type) {
        return vec!["out".into()];
    }
    if let Some(abs) = abstractions.iter().find(|a| a.name == component_type) {
        let mut names: Vec<String> = abs.outputs.keys().cloned().collect();
        names.sort();
        if names.is_empty() {
            vec!["out".into()]
        } else {
            names
        }
    } else {
        vec!["out".into()]
    }
}

// ── Component sizing ─────────────────────────────────────────────────────────

/// Compute the world-space size for a component based on its type.
pub fn component_size(
    component_type: &str,
    abstractions: &[ProjectAbstraction],
) -> egui::Vec2 {
    let input_count = input_port_names(component_type, abstractions).len();
    let output_count = output_port_names(component_type, abstractions).len();
    let port_count = input_count.max(output_count).max(1);
    let needed_height = (port_count as f32 + 1.0) * PORT_SPACING;

    if let Some(abs) = abstractions.iter().find(|a| a.name == component_type) {
        egui::vec2(abs.width, abs.height.max(needed_height))
    } else {
        egui::vec2(COMPONENT_WIDTH, COMPONENT_HEIGHT.max(needed_height))
    }
}

// ── Port geometry ────────────────────────────────────────────────────────────

/// Returns (screen_position, port_label) for each input port of a component.
pub fn input_port_positions(
    component_rect: egui::Rect,
    component_type: &str,
    abstractions: &[ProjectAbstraction],
) -> Vec<(egui::Pos2, String)> {
    let port_names = input_port_names(component_type, abstractions);
    let port_count = port_names.len() as f32;
    port_names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let y = component_rect.min.y
                + component_rect.height() * (i as f32 + 1.0) / (port_count + 1.0);
            (egui::pos2(component_rect.min.x, y), name)
        })
        .collect()
}

/// Returns (screen_position, output_name) for each output port of a component.
pub fn output_port_positions(
    component_rect: egui::Rect,
    component_type: &str,
    abstractions: &[ProjectAbstraction],
) -> Vec<(egui::Pos2, String)> {
    let port_names = output_port_names(component_type, abstractions);
    let count = port_names.len().max(1) as f32;
    port_names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let y = component_rect.min.y
                + component_rect.height() * (i as f32 + 1.0) / (count + 1.0);
            (egui::pos2(component_rect.max.x, y), name)
        })
        .collect()
}

// ── Endpoint position resolution ─────────────────────────────────────────────

/// World position for an abstraction input pin when editing inside an abstraction.
pub fn abstraction_input_pin_world_pos(input_index: usize) -> [f32; 2] {
    [-40.0, 40.0 + input_index as f32 * 40.0]
}

/// Resolve an endpoint to a screen-space position.
pub fn endpoint_screen_pos(
    endpoint: &Endpoint,
    component_ids: &[String],
    component_rects: &[egui::Rect],
    component_types: &[String],
    abstractions: &[ProjectAbstraction],
    net: &WireNet,
    canvas_origin: egui::Pos2,
    zoom: f32,
    editing_abstraction: Option<&ProjectAbstraction>,
) -> Option<egui::Pos2> {
    match endpoint {
        Endpoint::Port {
            component_id,
            port_name,
        } => {
            // Handle INPUT: pseudo-components (abstraction inputs)
            if let Some(input_name) = component_id.strip_prefix("INPUT:") {
                if let Some(abs) = editing_abstraction {
                    let idx = abs.inputs.iter().position(|i| i == input_name)?;
                    let [wx, wy] = abstraction_input_pin_world_pos(idx);
                    return Some(canvas_origin + egui::vec2(wx, wy) * zoom);
                }
                return None;
            }

            let comp_idx = component_ids.iter().position(|id| id == component_id)?;
            let rect = component_rects[comp_idx];
            let comp_type = &component_types[comp_idx];

            // Check if it's an output port
            let out_names = output_port_names(comp_type, abstractions);
            if let Some(idx) = out_names.iter().position(|n| n == port_name) {
                let count = out_names.len().max(1) as f32;
                let y = rect.min.y + rect.height() * (idx as f32 + 1.0) / (count + 1.0);
                return Some(egui::pos2(rect.max.x, y));
            }

            // Check if it's an input port
            let in_names = input_port_names(comp_type, abstractions);
            if let Some(idx) = in_names.iter().position(|n| n == port_name) {
                let count = in_names.len() as f32;
                let y = rect.min.y + rect.height() * (idx as f32 + 1.0) / (count + 1.0);
                return Some(egui::pos2(rect.min.x, y));
            }

            None
        }
        Endpoint::Junction { junction_id } => {
            let junction = net.junctions.iter().find(|j| j.id == *junction_id)?;
            Some(canvas_origin + egui::vec2(junction.pos[0], junction.pos[1]) * zoom)
        }
    }
}

// ── Geometry helpers ─────────────────────────────────────────────────────────

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
