use eframe::egui;
use std::collections::HashMap;

use diycpu::types::{is_primitive, AbstractionDef, ComponentDef};

use super::app::{App, WireState};
use super::style::*;

// ── Drawing helper functions (pure, no state mutation) ──────────────────────

fn draw_grid_background(
    painter: &egui::Painter,
    canvas_rect: egui::Rect,
    canvas_offset: egui::Vec2,
    zoom: f32,
) {
    let grid_line_color = egui::Color32::from_gray(40);
    let scaled_grid_spacing = GRID_SPACING * zoom;

    // Don't draw grid if zoomed out too far (lines would be too dense)
    if scaled_grid_spacing <= 4.0 {
        return;
    }

    let offset_x = canvas_offset.x.rem_euclid(scaled_grid_spacing);
    let offset_y = canvas_offset.y.rem_euclid(scaled_grid_spacing);

    // Vertical grid lines
    let mut x = canvas_rect.min.x + offset_x;
    while x < canvas_rect.max.x {
        painter.line_segment(
            [
                egui::pos2(x, canvas_rect.min.y),
                egui::pos2(x, canvas_rect.max.y),
            ],
            egui::Stroke::new(0.5, grid_line_color),
        );
        x += scaled_grid_spacing;
    }

    // Horizontal grid lines
    let mut y = canvas_rect.min.y + offset_y;
    while y < canvas_rect.max.y {
        painter.line_segment(
            [
                egui::pos2(canvas_rect.min.x, y),
                egui::pos2(canvas_rect.max.x, y),
            ],
            egui::Stroke::new(0.5, grid_line_color),
        );
        y += scaled_grid_spacing;
    }
}

/// Determine the color for a wire based on simulation state and selection.
fn wire_color_for_value(
    source_name: &str,
    current_step_values: Option<&HashMap<String, bool>>,
    is_simulation_active: bool,
    is_highlighted: bool,
) -> egui::Color32 {
    if is_simulation_active {
        match current_step_values.and_then(|values| values.get(source_name).copied()) {
            Some(true) => WIRE_HIGH,
            Some(false) => WIRE_LOW,
            None => WIRE_UNKNOWN,
        }
    } else if is_highlighted {
        egui::Color32::from_rgb(255, 255, 100)
    } else {
        WIRE_UNKNOWN
    }
}

/// Draw a wire between two ports, using user waypoints if available,
/// otherwise falling back to Manhattan routing.
fn draw_wire_with_waypoints(
    painter: &egui::Painter,
    source_position: egui::Pos2,
    destination_position: egui::Pos2,
    user_waypoints: Option<&Vec<[f32; 2]>>,
    color: egui::Color32,
    thickness: f32,
    zoom: f32,
    canvas_origin: egui::Pos2,
    draw_handles: bool,
) {
    let path: Vec<egui::Pos2> =
        if let Some(wps) = user_waypoints.filter(|w| !w.is_empty()) {
            // User-defined waypoints: source → wp1 → wp2 → ... → dest
            let mut points = vec![source_position];
            for &[x, y] in wps {
                points.push(canvas_origin + egui::vec2(x, y) * zoom);
            }
            points.push(destination_position);
            points
        } else {
            // Default Manhattan routing
            let midpoint_x = (source_position.x + destination_position.x) / 2.0;
            vec![
                source_position,
                egui::pos2(midpoint_x, source_position.y),
                egui::pos2(midpoint_x, destination_position.y),
                destination_position,
            ]
        };

    for segment in path.windows(2) {
        painter.line_segment(
            [segment[0], segment[1]],
            egui::Stroke::new(thickness, color),
        );
    }

    // Draw small circles at the endpoints
    let endpoint_radius = PORT_RADIUS * zoom * 0.8;
    painter.circle_filled(source_position, endpoint_radius, color);
    painter.circle_filled(destination_position, endpoint_radius, color);

    // Draw draggable waypoint handles (only in design mode)
    if draw_handles {
        if let Some(wps) = user_waypoints.filter(|w| !w.is_empty()) {
            let handle_size = WAYPOINT_HANDLE_SIZE * zoom;
            for &[x, y] in wps {
                let screen_pos = canvas_origin + egui::vec2(x, y) * zoom;
                let handle_rect = egui::Rect::from_center_size(
                    screen_pos,
                    egui::vec2(handle_size, handle_size),
                );
                // Highlight on hover (using a dummy check since we don't have hover_pos here)
                painter.rect_filled(handle_rect, 1.0, WAYPOINT_COLOR);
                painter.rect_stroke(
                    handle_rect,
                    1.0,
                    egui::Stroke::new(0.5 * zoom, WAYPOINT_HOVER_COLOR),
                    egui::StrokeKind::Outside,
                );
            }
        }
    }
}

/// Draw all wires connecting components based on their input maps.
fn draw_all_wires(
    painter: &egui::Painter,
    components: &[ComponentDef],
    component_rects: &[egui::Rect],
    abstractions: &[AbstractionDef],
    selected_component: Option<usize>,
    current_step_values: Option<&HashMap<String, bool>>,
    is_simulation_active: bool,
    zoom: f32,
    canvas_origin: egui::Pos2,
) {
    for (destination_index, destination_component) in components.iter().enumerate() {
        let Some(ref inputs) = destination_component.inputs else {
            continue;
        };

        let destination_rect = component_rects[destination_index];
        let input_keys = sorted_input_keys(destination_component);
        let input_port_count = input_keys.len() as f32;

        for (port_index, input_key) in input_keys.iter().enumerate() {
            let source_name = &inputs[input_key];
            if source_name.is_empty() {
                continue;
            }

            // Parse dot-notation: "inv.out" → instance="inv", output="out"
            let (instance_name, output_name) = if source_name.contains('.') {
                let parts: Vec<&str> = source_name.splitn(2, '.').collect();
                (parts[0], Some(parts[1]))
            } else {
                (source_name.as_str(), None)
            };

            // Find the source component by name
            let source_index = components
                .iter()
                .position(|c| c.name == instance_name);
            let Some(source_index) = source_index else {
                continue;
            };

            // Resolve the correct output port position
            let source_rect = component_rects[source_index];
            let source_position = if let Some(out_name) = output_name {
                let ports = output_port_positions(
                    source_rect,
                    &components[source_index],
                    abstractions,
                );
                ports
                    .iter()
                    .find(|(_, name)| name == out_name)
                    .map(|(pos, _)| *pos)
                    .unwrap_or_else(|| output_port_position(source_rect))
            } else {
                // For primitives or single-output, use the first (only) output port
                let ports = output_port_positions(
                    source_rect,
                    &components[source_index],
                    abstractions,
                );
                ports
                    .first()
                    .map(|(pos, _)| *pos)
                    .unwrap_or_else(|| output_port_position(source_rect))
            };

            // Calculate the destination port position
            let destination_y = destination_rect.min.y
                + destination_rect.height() * (port_index as f32 + 1.0)
                    / (input_port_count + 1.0);
            let destination_position = egui::pos2(destination_rect.min.x, destination_y);

            // Determine wire color based on simulation state or selection
            let is_wire_highlighted = selected_component == Some(destination_index)
                || selected_component == Some(source_index);
            let wire_color = wire_color_for_value(
                source_name,
                current_step_values,
                is_simulation_active,
                is_wire_highlighted,
            );
            let wire_thickness = if is_simulation_active {
                2.0 * zoom
            } else {
                1.5 * zoom
            };

            // Get user waypoints if any
            let user_waypoints = destination_component
                .wire_waypoints
                .as_ref()
                .and_then(|wps| wps.get(input_key));

            draw_wire_with_waypoints(
                painter,
                source_position,
                destination_position,
                user_waypoints,
                wire_color,
                wire_thickness,
                zoom,
                canvas_origin,
                !is_simulation_active,
            );
        }
    }
}

/// Draw the in-progress wire from a source output port to the current pointer position.
fn draw_wire_in_progress(
    painter: &egui::Painter,
    wire_state: &WireState,
    components: &[ComponentDef],
    component_rects: &[egui::Rect],
    abstractions: &[AbstractionDef],
    hover_position: Option<egui::Pos2>,
    zoom: f32,
) {
    let Some(source_index) = wire_state.source_component_index else {
        return;
    };
    if source_index >= component_rects.len() {
        return;
    }

    // Find the correct output port position based on which port was clicked
    let source_position = if let Some(ref out_name) = wire_state.source_output_name {
        let ports = output_port_positions(
            component_rects[source_index],
            &components[source_index],
            abstractions,
        );
        ports
            .iter()
            .find(|(_, name)| name == out_name)
            .map(|(pos, _)| *pos)
            .unwrap_or_else(|| output_port_position(component_rects[source_index]))
    } else {
        output_port_position(component_rects[source_index])
    };

    if let Some(pointer) = hover_position {
        painter.line_segment(
            [source_position, pointer],
            egui::Stroke::new(2.0, PORT_HOVER_COLOR),
        );
        painter.circle_filled(source_position, PORT_RADIUS * zoom, PORT_HOVER_COLOR);
    }
}

/// Draw a single component: background, border, label, badges, and ports.
fn draw_single_component(
    painter: &egui::Painter,
    component_rect: egui::Rect,
    component: &ComponentDef,
    abstractions: &[AbstractionDef],
    is_selected: bool,
    simulation_value: Option<bool>,
    is_simulation_active: bool,
    hover_position: Option<egui::Pos2>,
    zoom: f32,
) {
    let component_color = color_for_component_type(&component.component_type);
    let label_font_size = (13.0 * zoom).max(7.0);
    let port_font_size = (10.0 * zoom).max(6.0);

    // Background fill — tinted by simulation value when active
    let fill_color = if is_simulation_active {
        match simulation_value {
            Some(true) => egui::Color32::from_rgba_premultiplied(40, 100, 40, 200),
            Some(false) => egui::Color32::from_rgba_premultiplied(30, 30, 30, 200),
            None => component_color.linear_multiply(0.3),
        }
    } else {
        component_color.linear_multiply(0.3)
    };
    painter.rect_filled(component_rect, 6.0 * zoom, fill_color);

    // Border stroke — thicker and white when selected
    let border_stroke = if is_selected {
        egui::Stroke::new(2.5 * zoom, egui::Color32::WHITE)
    } else {
        egui::Stroke::new(1.0 * zoom, component_color)
    };
    painter.rect_stroke(
        component_rect,
        6.0 * zoom,
        border_stroke,
        egui::StrokeKind::Outside,
    );

    // Component label (name and type)
    let label_text = format!("{}\n({})", component.name, component.component_type);
    painter.text(
        component_rect.center(),
        egui::Align2::CENTER_CENTER,
        &label_text,
        egui::FontId::proportional(label_font_size),
        egui::Color32::WHITE,
    );

    // Value badge in the top-right corner
    draw_component_badge(
        painter,
        component_rect,
        component,
        simulation_value,
        is_simulation_active,
        zoom,
    );

    // Draw a subtle double-border on abstraction instances to hint they can be drilled into
    if !is_primitive(&component.component_type) && component.component_type != "clk" {
        let inner_rect = component_rect.shrink(3.0 * zoom);
        painter.rect_stroke(
            inner_rect,
            4.0 * zoom,
            egui::Stroke::new(0.5 * zoom, component_color.linear_multiply(0.4)),
            egui::StrokeKind::Outside,
        );
    }

    // Input and output ports
    draw_component_ports(
        painter,
        component_rect,
        component,
        abstractions,
        component_color,
        hover_position,
        zoom,
        port_font_size,
    );
}

/// Draw the simulation value or init value badge on a component.
fn draw_component_badge(
    painter: &egui::Painter,
    component_rect: egui::Rect,
    component: &ComponentDef,
    simulation_value: Option<bool>,
    is_simulation_active: bool,
    zoom: f32,
) {
    if is_simulation_active {
        if let Some(value) = simulation_value {
            let badge_text = if value { "1" } else { "0" };
            let badge_color = if value { WIRE_HIGH } else { WIRE_LOW };
            painter.text(
                component_rect.right_top() + egui::vec2(-10.0 * zoom, 10.0 * zoom),
                egui::Align2::CENTER_CENTER,
                badge_text,
                egui::FontId::monospace((14.0 * zoom).max(8.0)),
                badge_color,
            );
        }
    } else if component.component_type == "dff" {
        // Init value badge for DFFs (shown when simulation is not active)
        if let Some(init_value) = component.init {
            let badge_text = if init_value { "1" } else { "0" };
            painter.text(
                component_rect.right_top() + egui::vec2(-8.0 * zoom, 8.0 * zoom),
                egui::Align2::CENTER_CENTER,
                badge_text,
                egui::FontId::monospace((11.0 * zoom).max(6.0)),
                egui::Color32::YELLOW,
            );
        }
    }
}

/// Draw the input and output port circles and labels for a component.
fn draw_component_ports(
    painter: &egui::Painter,
    component_rect: egui::Rect,
    component: &ComponentDef,
    abstractions: &[AbstractionDef],
    component_color: egui::Color32,
    hover_position: Option<egui::Pos2>,
    zoom: f32,
    port_font_size: f32,
) {
    let port_hit_radius = PORT_RADIUS * zoom * 2.5;

    // Input ports (left side)
    for (port_position, port_label) in input_port_positions(component_rect, component) {
        let is_hovering = hover_position
            .map(|hp| hp.distance(port_position) < port_hit_radius)
            .unwrap_or(false);
        let port_color = if is_hovering {
            PORT_HOVER_COLOR
        } else {
            component_color
        };
        painter.circle_filled(port_position, PORT_RADIUS * zoom, port_color);
        painter.text(
            port_position + egui::vec2(8.0 * zoom, 0.0),
            egui::Align2::LEFT_CENTER,
            &port_label,
            egui::FontId::proportional(port_font_size),
            egui::Color32::from_gray(200),
        );
    }

    // Output ports (right side) — multiple for abstractions
    let output_ports = output_port_positions(component_rect, component, abstractions);
    for (output_position, output_label) in &output_ports {
        let is_output_hovering = hover_position
            .map(|hp| hp.distance(*output_position) < port_hit_radius)
            .unwrap_or(false);
        let output_color = if is_output_hovering {
            PORT_HOVER_COLOR
        } else {
            component_color
        };
        painter.circle_filled(*output_position, PORT_RADIUS * zoom, output_color);
        painter.text(
            *output_position + egui::vec2(-8.0 * zoom, 0.0),
            egui::Align2::RIGHT_CENTER,
            output_label,
            egui::FontId::proportional(port_font_size),
            egui::Color32::from_gray(200),
        );
    }
}

/// Draw input/output pin labels when editing inside an abstraction.
fn draw_abstraction_pins(
    painter: &egui::Painter,
    abstraction: &AbstractionDef,
    canvas_origin: egui::Pos2,
    canvas_rect: egui::Rect,
    zoom: f32,
) {
    let font = egui::FontId::proportional((12.0 * zoom).max(8.0));
    let pin_radius = 8.0 * zoom;

    // Input pins along the left edge
    for (i, input_name) in abstraction.inputs.iter().enumerate() {
        let y = canvas_rect.min.y + 60.0 + (i as f32 + 1.0) * 40.0 * zoom;
        let x = canvas_origin.x - 20.0 * zoom;
        let pos = egui::pos2(x.max(canvas_rect.min.x + 10.0), y);
        painter.circle_filled(pos, pin_radius, INPUT_PIN_COLOR);
        painter.text(
            pos + egui::vec2(pin_radius + 4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            input_name,
            font.clone(),
            INPUT_PIN_COLOR,
        );
    }

    // Output pins along the right edge
    let mut output_names: Vec<&String> = abstraction.outputs.keys().collect();
    output_names.sort();
    for (i, output_name) in output_names.iter().enumerate() {
        let y = canvas_rect.min.y + 60.0 + (i as f32 + 1.0) * 40.0 * zoom;
        let x = canvas_rect.max.x - 20.0;
        let target = &abstraction.outputs[*output_name];
        let label = format!("{output_name} <- {target}");
        painter.circle_filled(egui::pos2(x, y), pin_radius, OUTPUT_PIN_COLOR);
        painter.text(
            egui::pos2(x - pin_radius - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            &label,
            font.clone(),
            OUTPUT_PIN_COLOR,
        );
    }
}

/// Draw the zoom percentage indicator in the bottom-right corner of the canvas.
fn draw_zoom_indicator(painter: &egui::Painter, canvas_rect: egui::Rect, zoom: f32) {
    painter.text(
        canvas_rect.right_bottom() + egui::vec2(-10.0, -10.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{:.0}%", zoom * 100.0),
        egui::FontId::proportional(12.0),
        egui::Color32::from_gray(100),
    );
}

/// Find the topmost component whose screen rect contains the given position.
/// Searches from last (top) to first (bottom) for correct z-order.
fn find_component_at_position(
    screen_position: egui::Pos2,
    component_rects: &[egui::Rect],
) -> Option<usize> {
    for index in (0..component_rects.len()).rev() {
        if component_rects[index].contains(screen_position) {
            return Some(index);
        }
    }
    None
}

// ── App methods for canvas interaction ──────────────────────────────────────

impl App {
    /// Draw the central canvas with all components, wires, and handle interactions.
    pub fn draw_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let canvas_rect = response.rect;

        // Guard: if editing an abstraction that was deleted, go back to main
        if let Some(abs_idx) = self.editing_abstraction_index {
            if abs_idx >= self.circuit.abstractions.len() {
                self.editing_abstraction_index = None;
            }
        }

        // Phase 1: Handle zoom and panning (modifies self.zoom and self.canvas_offset)
        self.handle_scroll_zoom(ctx, canvas_rect);
        self.handle_canvas_panning(&response, ctx);

        // Phase 2: Compute layout using active components (main circuit or abstraction)
        let canvas_origin = canvas_rect.min + self.canvas_offset;
        let components: Vec<ComponentDef> = self.active_components().clone();
        let abstractions: Vec<AbstractionDef> = self.circuit.abstractions.clone();
        let component_count = components.len();
        let component_rects: Vec<egui::Rect> = (0..component_count)
            .map(|index| self.component_screen_rect(index, canvas_origin))
            .collect();
        let hover_position = ctx.input(|i| i.pointer.hover_pos());
        let is_simulation_active = self.simulation_results.is_some()
            && self.editing_abstraction_index.is_none();

        // Clone the current step values to avoid holding a borrow on self
        let current_step_values: Option<HashMap<String, bool>> = self
            .simulation_results
            .as_ref()
            .and_then(|results| results.get(self.simulation_step))
            .cloned();

        // Phase 3: Draw everything (pure functions, no state mutation)
        draw_grid_background(&painter, canvas_rect, self.canvas_offset, self.zoom);

        // Draw abstraction input/output pins when editing inside an abstraction
        if let Some(abs_idx) = self.editing_abstraction_index {
            if abs_idx < self.circuit.abstractions.len() {
                draw_abstraction_pins(
                    &painter,
                    &self.circuit.abstractions[abs_idx],
                    canvas_origin,
                    canvas_rect,
                    self.zoom,
                );
            }
        }

        draw_all_wires(
            &painter,
            &components,
            &component_rects,
            &abstractions,
            self.selected_component,
            current_step_values.as_ref(),
            is_simulation_active,
            self.zoom,
            canvas_origin,
        );

        draw_wire_in_progress(
            &painter,
            &self.wire_state,
            &components,
            &component_rects,
            &abstractions,
            hover_position,
            self.zoom,
        );

        for component_index in 0..component_count {
            let component = &components[component_index];
            let sim_value = current_step_values
                .as_ref()
                .and_then(|values| values.get(&component.name).copied());
            draw_single_component(
                &painter,
                component_rects[component_index],
                component,
                &abstractions,
                self.selected_component == Some(component_index),
                sim_value,
                is_simulation_active,
                hover_position,
                self.zoom,
            );
        }

        draw_zoom_indicator(&painter, canvas_rect, self.zoom);

        // Show tooltip on component hover
        if let Some(hover_pos) = hover_position {
            if self.wire_state.source_component_index.is_none()
                && self.dragging_component.is_none()
            {
                if let Some(idx) = find_component_at_position(hover_pos, &component_rects) {
                    let comp = &components[idx];
                    let mut tip = format!("{} ({})", comp.name, comp.component_type);
                    if let Some(ref inputs) = comp.inputs {
                        let mut keys: Vec<_> = inputs.keys().collect();
                        keys.sort();
                        for k in keys {
                            let v = &inputs[k];
                            if !v.is_empty() {
                                tip.push_str(&format!("\n  {k}: {v}"));
                            }
                        }
                    }
                    if let Some(ref vals) = current_step_values {
                        if let Some(&v) = vals.get(&comp.name) {
                            tip.push_str(&format!("\n  = {}", if v { "1" } else { "0" }));
                        }
                    }
                    response.clone().on_hover_text_at_pointer(&tip);
                }
            }
        }

        // Phase 4: Handle click and drag interactions (modifies self)
        self.handle_canvas_clicks(&response, &component_rects, canvas_origin, ctx);
        self.handle_component_dragging(&response, &component_rects, canvas_origin, ctx);
    }

    /// Handle mouse scroll wheel to zoom in/out, centered on the pointer position.
    fn handle_scroll_zoom(&mut self, ctx: &egui::Context, canvas_rect: egui::Rect) {
        let scroll_delta = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta == 0.0 {
            return;
        }

        let pointer_is_in_canvas = canvas_rect
            .contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default()));
        if !pointer_is_in_canvas {
            return;
        }

        let zoom_factor = if scroll_delta > 0.0 { 1.02 } else { 1.0 / 1.02 };
        let new_zoom = (self.zoom * zoom_factor).clamp(MIN_ZOOM, MAX_ZOOM);

        // Zoom towards the pointer position so the point under the cursor stays fixed
        if let Some(pointer_position) = ctx.input(|i| i.pointer.hover_pos()) {
            let world_position_before_zoom =
                (pointer_position - canvas_rect.min - self.canvas_offset) / self.zoom;
            self.zoom = new_zoom;
            let screen_position_after_zoom = world_position_before_zoom * self.zoom;
            self.canvas_offset = pointer_position - canvas_rect.min - screen_position_after_zoom;
        } else {
            self.zoom = new_zoom;
        }
    }

    /// Handle middle/right mouse button drag to pan the canvas.
    fn handle_canvas_panning(&mut self, response: &egui::Response, ctx: &egui::Context) {
        // Start panning on middle or right mouse button drag
        if let Some(pointer_position) = response.interact_pointer_pos() {
            if response.drag_started_by(egui::PointerButton::Middle)
                || response.drag_started_by(egui::PointerButton::Secondary)
            {
                self.is_panning = true;
                self.last_pan_pointer_position = Some(pointer_position);
            }
        }

        // Continue panning while button is held
        if self.is_panning {
            if let Some(current_pointer) = ctx.pointer_interact_pos() {
                if let Some(previous_pointer) = self.last_pan_pointer_position {
                    self.canvas_offset += current_pointer - previous_pointer;
                }
                self.last_pan_pointer_position = Some(current_pointer);
            }

            let pan_button_released = ctx.input(|i| {
                i.pointer.button_released(egui::PointerButton::Middle)
                    || i.pointer.button_released(egui::PointerButton::Secondary)
            });
            if pan_button_released {
                self.is_panning = false;
                self.last_pan_pointer_position = None;
            }
        }
    }

    /// Handle left-click interactions: port wiring, component selection, double-click.
    fn handle_canvas_clicks(
        &mut self,
        response: &egui::Response,
        component_rects: &[egui::Rect],
        canvas_origin: egui::Pos2,
        ctx: &egui::Context,
    ) {
        let port_hit_radius = PORT_RADIUS * self.zoom * 2.5;

        // Handle right-click to delete waypoints
        if response.secondary_clicked() {
            if let Some(click_pos) = response.interact_pointer_pos() {
                self.handle_waypoint_delete(click_pos, component_rects, canvas_origin);
            }
        }

        // Handle double-click: add waypoint to wire, or enter abstraction
        let double_clicked = ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        if double_clicked {
            if let Some(click_pos) = ctx.input(|i| i.pointer.interact_pos()) {
                // First check if double-clicking an abstraction instance to drill in
                if self.wire_state.source_component_index.is_none() {
                    if let Some(comp_idx) = find_component_at_position(click_pos, component_rects) {
                        let comp_type = self.active_components()[comp_idx].component_type.clone();
                        if !is_primitive(&comp_type) {
                            if let Some(abs_idx) = self.circuit.abstractions.iter().position(|a| a.name == comp_type) {
                                self.editing_abstraction_index = Some(abs_idx);
                                self.selected_component = None;
                                self.canvas_offset = egui::Vec2::ZERO;
                                self.status_message = format!("Editing abstraction: {comp_type}");
                                return;
                            }
                        }
                    }
                }
                // Otherwise try to add a waypoint to a nearby wire
                self.handle_waypoint_add(click_pos, component_rects, canvas_origin);
            }
        }

        if !response.clicked() {
            return;
        }

        let Some(click_position) = response.interact_pointer_pos() else {
            return;
        };

        if self.wire_state.source_component_index.is_some() {
            // We're in wire mode — look for an input port to connect to
            self.handle_wire_destination_click(click_position, component_rects, port_hit_radius);
        } else {
            // Check if clicking an output port to start wiring
            let started_wire = self.try_start_wire_from_output_port(
                click_position,
                component_rects,
                port_hit_radius,
            );
            if !started_wire {
                // Normal click — select the component under the cursor
                self.selected_component =
                    find_component_at_position(click_position, component_rects);
            }
        }
    }

    /// Attempt to connect a wire to an input port at the click position.
    fn handle_wire_destination_click(
        &mut self,
        click_position: egui::Pos2,
        component_rects: &[egui::Rect],
        port_hit_radius: f32,
    ) {
        let component_count = self.active_components().len();
        let mut connected = false;

        'outer: for destination_index in 0..component_count {
            let ports = input_port_positions(
                component_rects[destination_index],
                &self.active_components()[destination_index],
            );

            for (port_position, port_key) in &ports {
                if click_position.distance(*port_position) < port_hit_radius {
                    let source_index = self.wire_state.source_component_index.unwrap();
                    let source_reference =
                        self.build_source_reference(source_index);
                    self.save_undo_checkpoint();

                    let destination_component =
                        &mut self.active_components_mut()[destination_index];
                    if destination_component.inputs.is_none() {
                        destination_component.inputs = Some(HashMap::new());
                    }
                    destination_component
                        .inputs
                        .as_mut()
                        .unwrap()
                        .insert(port_key.clone(), source_reference.clone());

                    let dest_name = destination_component.name.clone();
                    self.wire_state.source_component_index = None;
                    self.wire_state.source_output_name = None;
                    self.status_message = format!(
                        "Connected {source_reference} -> {dest_name}.{port_key}",
                    );
                    connected = true;
                    break 'outer;
                }
            }
        }

        if !connected {
            self.wire_state.source_component_index = None;
            self.wire_state.source_output_name = None;
            self.status_message = "Wire cancelled".into();
        }
    }

    /// Build the correct source reference for a component when creating a wire.
    /// For primitives (clk, nand, dff), the reference is just the component name.
    /// For abstraction instances, it's "instance_name.output_name" (dot-notation).
    fn build_source_reference(&self, source_index: usize) -> String {
        let source_component = &self.active_components()[source_index];

        if is_primitive(&source_component.component_type) {
            return source_component.name.clone();
        }

        // Use the stored output name from WireState if available
        if let Some(ref output_name) = self.wire_state.source_output_name {
            return format!("{}.{}", source_component.name, output_name);
        }

        // Fallback: pick the first output from the abstraction definition
        let abstraction = self
            .circuit
            .abstractions
            .iter()
            .find(|a| a.name == source_component.component_type);

        if let Some(abstraction) = abstraction {
            let mut output_names: Vec<&String> = abstraction.outputs.keys().collect();
            output_names.sort();
            if let Some(output_name) = output_names.first() {
                return format!("{}.{}", source_component.name, output_name);
            }
        }

        source_component.name.clone()
    }

    /// Check if the click is on an output port and start wire drawing mode.
    /// Returns true if a wire was started.
    fn try_start_wire_from_output_port(
        &mut self,
        click_position: egui::Pos2,
        component_rects: &[egui::Rect],
        port_hit_radius: f32,
    ) -> bool {
        let component_count = component_rects.len();
        let components = self.active_components().clone();
        let abstractions = self.circuit.abstractions.clone();

        for component_index in (0..component_count).rev() {
            let output_ports = output_port_positions(
                component_rects[component_index],
                &components[component_index],
                &abstractions,
            );
            for (port_pos, port_name) in &output_ports {
                if click_position.distance(*port_pos) < port_hit_radius {
                    self.wire_state.source_component_index = Some(component_index);
                    self.wire_state.source_output_name = Some(port_name.clone());
                    self.status_message = format!(
                        "Wiring from '{}'.{port_name} — click an input port",
                        components[component_index].name
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Handle left-click drag to move components or waypoints on the canvas.
    fn handle_component_dragging(
        &mut self,
        response: &egui::Response,
        component_rects: &[egui::Rect],
        canvas_origin: egui::Pos2,
        ctx: &egui::Context,
    ) {
        let component_count = self.active_components().len();

        // Start a new drag on left mouse button
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pointer_position) = response.interact_pointer_pos() {
                let port_hit_radius = PORT_RADIUS * self.zoom * 2.5;

                // First: check if starting a waypoint drag
                if self.try_start_waypoint_drag(pointer_position, canvas_origin) {
                    return;
                }

                // Don't start dragging if the pointer is on a port
                let components = self.active_components().clone();
                let abstractions = self.circuit.abstractions.clone();
                let is_pointer_on_port = (0..component_count).any(|index| {
                    let out_ports = output_port_positions(
                        component_rects[index],
                        &components[index],
                        &abstractions,
                    );
                    if out_ports.iter().any(|(pos, _)| pointer_position.distance(*pos) < port_hit_radius) {
                        return true;
                    }
                    let in_ports = input_port_positions(
                        component_rects[index],
                        &components[index],
                    );
                    in_ports
                        .iter()
                        .any(|(pt, _)| pointer_position.distance(*pt) < port_hit_radius)
                });

                if !is_pointer_on_port {
                    if let Some(component_index) =
                        find_component_at_position(pointer_position, component_rects)
                    {
                        self.selected_component = Some(component_index);
                        self.dragging_component = Some(component_index);
                        self.drag_offset =
                            pointer_position - component_rects[component_index].min;
                    }
                }
            }
        }

        // Continue dragging a waypoint
        if let Some((comp_idx, ref input_key, wp_idx)) = self.dragging_waypoint {
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(pointer_position) = ctx.pointer_interact_pos() {
                    let world_pos =
                        (pointer_position - canvas_origin.to_vec2()) / self.zoom;
                    let snapped_x = snap_to_grid(world_pos.x);
                    let snapped_y = snap_to_grid(world_pos.y);
                    let key = input_key.clone();
                    if let Some(wps) = self.active_components_mut()[comp_idx]
                        .wire_waypoints
                        .as_mut()
                        .and_then(|m| m.get_mut(&key))
                    {
                        if wp_idx < wps.len() {
                            wps[wp_idx] = [snapped_x, snapped_y];
                        }
                    }
                }
            }
            if response.drag_stopped() {
                self.save_undo_checkpoint();
                self.dragging_waypoint = None;
            }
            return;
        }

        // Continue dragging a component — update position with grid snapping
        if let Some(drag_index) = self.dragging_component {
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(pointer_position) = ctx.pointer_interact_pos() {
                    let new_world_position =
                        (pointer_position - self.drag_offset - canvas_origin.to_vec2()) / self.zoom;
                    let snapped_x = snap_to_grid(new_world_position.x);
                    let snapped_y = snap_to_grid(new_world_position.y);
                    self.active_components_mut()[drag_index].pos = Some([snapped_x, snapped_y]);
                }
            }
            if response.drag_stopped() {
                self.save_undo_checkpoint();
                self.dragging_component = None;
            }
        }
    }

    /// Try to start dragging a waypoint handle. Returns true if a waypoint was found.
    fn try_start_waypoint_drag(
        &mut self,
        pointer_position: egui::Pos2,
        canvas_origin: egui::Pos2,
    ) -> bool {
        let hit_radius = WAYPOINT_HANDLE_SIZE * self.zoom * 3.0;
        let components = self.active_components().clone();

        for (comp_idx, component) in components.iter().enumerate() {
            if let Some(ref waypoint_map) = component.wire_waypoints {
                for (input_key, waypoints) in waypoint_map {
                    for (wp_idx, &[x, y]) in waypoints.iter().enumerate() {
                        let screen_pos = canvas_origin + egui::vec2(x, y) * self.zoom;
                        if pointer_position.distance(screen_pos) < hit_radius {
                            self.dragging_waypoint =
                                Some((comp_idx, input_key.clone(), wp_idx));
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Handle double-click on a wire to add a new waypoint.
    fn handle_waypoint_add(
        &mut self,
        click_position: egui::Pos2,
        component_rects: &[egui::Rect],
        canvas_origin: egui::Pos2,
    ) {
        let hit_distance = 10.0 * self.zoom;
        let components = self.active_components().clone();
        let abstractions = self.circuit.abstractions.clone();

        for (dest_idx, dest_component) in components.iter().enumerate() {
            let Some(ref inputs) = dest_component.inputs else { continue };
            let dest_rect = component_rects[dest_idx];
            let input_keys = sorted_input_keys(dest_component);
            let input_port_count = input_keys.len() as f32;

            for (port_idx, input_key) in input_keys.iter().enumerate() {
                let source_name = &inputs[input_key];
                if source_name.is_empty() { continue; }

                let (instance_name, output_name) = if source_name.contains('.') {
                    let parts: Vec<&str> = source_name.splitn(2, '.').collect();
                    (parts[0], Some(parts[1]))
                } else {
                    (source_name.as_str(), None)
                };

                let source_index = components.iter().position(|c| c.name == instance_name);
                let Some(source_index) = source_index else { continue };

                let source_rect = component_rects[source_index];
                let source_pos = if let Some(out_name) = output_name {
                    let ports = output_port_positions(source_rect, &components[source_index], &abstractions);
                    ports.iter().find(|(_, n)| n == out_name).map(|(p, _)| *p)
                        .unwrap_or_else(|| output_port_position(source_rect))
                } else {
                    let ports = output_port_positions(source_rect, &components[source_index], &abstractions);
                    ports.first().map(|(p, _)| *p).unwrap_or_else(|| output_port_position(source_rect))
                };

                let dest_y = dest_rect.min.y
                    + dest_rect.height() * (port_idx as f32 + 1.0) / (input_port_count + 1.0);
                let dest_pos = egui::pos2(dest_rect.min.x, dest_y);

                // Build path including existing waypoints
                let existing_wps = dest_component.wire_waypoints.as_ref()
                    .and_then(|m| m.get(input_key));

                let mut path = vec![source_pos];
                if let Some(wps) = existing_wps {
                    for &[x, y] in wps {
                        path.push(canvas_origin + egui::vec2(x, y) * self.zoom);
                    }
                } else {
                    // Manhattan routing points
                    let mid_x = (source_pos.x + dest_pos.x) / 2.0;
                    path.push(egui::pos2(mid_x, source_pos.y));
                    path.push(egui::pos2(mid_x, dest_pos.y));
                }
                path.push(dest_pos);

                // Check if click is near any segment
                for seg_idx in 0..path.len() - 1 {
                    let dist = point_to_segment_distance(
                        click_position, path[seg_idx], path[seg_idx + 1],
                    );
                    if dist < hit_distance {
                        // Insert new waypoint in world coordinates
                        let world_pos = (click_position - canvas_origin.to_vec2()) / self.zoom;
                        let snapped_x = snap_to_grid(world_pos.x);
                        let snapped_y = snap_to_grid(world_pos.y);

                        self.save_undo_checkpoint();
                        let comp = &mut self.active_components_mut()[dest_idx];
                        let waypoint_map = comp.wire_waypoints.get_or_insert_with(HashMap::new);
                        let wp_list = waypoint_map.entry(input_key.clone()).or_insert_with(Vec::new);

                        // Determine insert position within the waypoint list
                        // seg_idx 0 is source→first_wp (or source→manhattan), so
                        // we insert at position = seg_idx for existing waypoints,
                        // or just push for the first one.
                        let insert_pos = if existing_wps.is_some() {
                            // seg_idx=0: before first wp → insert at 0
                            // seg_idx=1: between wp[0] and wp[1] → insert at 1
                            seg_idx.min(wp_list.len())
                        } else {
                            0
                        };
                        wp_list.insert(insert_pos, [snapped_x, snapped_y]);
                        self.status_message = "Waypoint added (drag to move, right-click to delete)".into();
                        return;
                    }
                }
            }
        }
    }

    /// Handle right-click to delete a waypoint handle.
    fn handle_waypoint_delete(
        &mut self,
        click_position: egui::Pos2,
        _component_rects: &[egui::Rect],
        canvas_origin: egui::Pos2,
    ) {
        let hit_radius = WAYPOINT_HANDLE_SIZE * self.zoom * 3.0;
        let components = self.active_components().clone();

        for (comp_idx, component) in components.iter().enumerate() {
            if let Some(ref waypoint_map) = component.wire_waypoints {
                for (input_key, waypoints) in waypoint_map {
                    for (wp_idx, &[x, y]) in waypoints.iter().enumerate() {
                        let screen_pos = canvas_origin + egui::vec2(x, y) * self.zoom;
                        if click_position.distance(screen_pos) < hit_radius {
                            self.save_undo_checkpoint();
                            let comp = &mut self.active_components_mut()[comp_idx];
                            if let Some(ref mut wm) = comp.wire_waypoints {
                                if let Some(wps) = wm.get_mut(input_key) {
                                    wps.remove(wp_idx);
                                    if wps.is_empty() {
                                        wm.remove(input_key);
                                    }
                                }
                                if wm.is_empty() {
                                    comp.wire_waypoints = None;
                                }
                            }
                            self.status_message = "Waypoint deleted".into();
                            return;
                        }
                    }
                }
            }
        }
    }
}
