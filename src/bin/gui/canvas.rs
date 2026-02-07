use eframe::egui;
use std::collections::HashMap;

use diycpu::types::{is_primitive, ComponentDef};

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

/// Draw a Manhattan-routed wire (horizontal-vertical-horizontal) between two ports.
fn draw_manhattan_wire(
    painter: &egui::Painter,
    source_position: egui::Pos2,
    destination_position: egui::Pos2,
    color: egui::Color32,
    thickness: f32,
    zoom: f32,
) {
    let midpoint_x = (source_position.x + destination_position.x) / 2.0;
    let waypoints = [
        source_position,
        egui::pos2(midpoint_x, source_position.y),
        egui::pos2(midpoint_x, destination_position.y),
        destination_position,
    ];

    for segment in waypoints.windows(2) {
        painter.line_segment(
            [segment[0], segment[1]],
            egui::Stroke::new(thickness, color),
        );
    }

    // Draw small circles at the endpoints
    let endpoint_radius = PORT_RADIUS * zoom * 0.8;
    painter.circle_filled(source_position, endpoint_radius, color);
    painter.circle_filled(destination_position, endpoint_radius, color);
}

/// Draw all wires connecting components based on their input maps.
fn draw_all_wires(
    painter: &egui::Painter,
    components: &[ComponentDef],
    component_rects: &[egui::Rect],
    selected_component: Option<usize>,
    current_step_values: Option<&HashMap<String, bool>>,
    is_simulation_active: bool,
    zoom: f32,
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

            // Find the source component by name. For abstraction outputs using
            // dot-notation (e.g. "inv.out"), match the part before the dot.
            let source_index = components
                .iter()
                .position(|c| c.name == *source_name)
                .or_else(|| {
                    let instance_name = source_name.split('.').next()?;
                    components.iter().position(|c| c.name == instance_name)
                });
            let Some(source_index) = source_index else {
                continue;
            };

            let source_rect = component_rects[source_index];
            let source_position = output_port_position(source_rect);

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

            draw_manhattan_wire(
                painter,
                source_position,
                destination_position,
                wire_color,
                wire_thickness,
                zoom,
            );
        }
    }
}

/// Draw the in-progress wire from a source output port to the current pointer position.
fn draw_wire_in_progress(
    painter: &egui::Painter,
    wire_state: &WireState,
    component_rects: &[egui::Rect],
    hover_position: Option<egui::Pos2>,
    zoom: f32,
) {
    let Some(source_index) = wire_state.source_component_index else {
        return;
    };
    if source_index >= component_rects.len() {
        return;
    }

    let source_position = output_port_position(component_rects[source_index]);
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

    // Input and output ports
    draw_component_ports(
        painter,
        component_rect,
        component,
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

    // Output port (right side)
    let output_position = output_port_position(component_rect);
    let is_output_hovering = hover_position
        .map(|hp| hp.distance(output_position) < port_hit_radius)
        .unwrap_or(false);
    let output_color = if is_output_hovering {
        PORT_HOVER_COLOR
    } else {
        component_color
    };
    painter.circle_filled(output_position, PORT_RADIUS * zoom, output_color);
    painter.text(
        output_position + egui::vec2(-8.0 * zoom, 0.0),
        egui::Align2::RIGHT_CENTER,
        "out",
        egui::FontId::proportional(port_font_size),
        egui::Color32::from_gray(200),
    );
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

        // Phase 1: Handle zoom and panning (modifies self.zoom and self.canvas_offset)
        self.handle_scroll_zoom(ctx, canvas_rect);
        self.handle_canvas_panning(&response, ctx);

        // Phase 2: Compute layout
        let canvas_origin = canvas_rect.min + self.canvas_offset;
        let component_count = self.circuit.components.len();
        let component_rects: Vec<egui::Rect> = (0..component_count)
            .map(|index| self.component_screen_rect(index, canvas_origin))
            .collect();
        let hover_position = ctx.input(|i| i.pointer.hover_pos());
        let is_simulation_active = self.simulation_results.is_some();

        // Clone the current step values to avoid holding a borrow on self
        let current_step_values: Option<HashMap<String, bool>> = self
            .simulation_results
            .as_ref()
            .and_then(|results| results.get(self.simulation_step))
            .cloned();

        // Phase 3: Draw everything (pure functions, no state mutation)
        draw_grid_background(&painter, canvas_rect, self.canvas_offset, self.zoom);

        draw_all_wires(
            &painter,
            &self.circuit.components,
            &component_rects,
            self.selected_component,
            current_step_values.as_ref(),
            is_simulation_active,
            self.zoom,
        );

        draw_wire_in_progress(
            &painter,
            &self.wire_state,
            &component_rects,
            hover_position,
            self.zoom,
        );

        for component_index in 0..component_count {
            let component = &self.circuit.components[component_index];
            let sim_value = current_step_values
                .as_ref()
                .and_then(|values| values.get(&component.name).copied());
            draw_single_component(
                &painter,
                component_rects[component_index],
                component,
                self.selected_component == Some(component_index),
                sim_value,
                is_simulation_active,
                hover_position,
                self.zoom,
            );
        }

        draw_zoom_indicator(&painter, canvas_rect, self.zoom);

        // Phase 4: Handle click and drag interactions (modifies self)
        self.handle_canvas_clicks(&response, &component_rects);
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

        let zoom_factor = if scroll_delta > 0.0 { 1.1 } else { 1.0 / 1.1 };
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

    /// Handle left-click interactions: port wiring and component selection.
    fn handle_canvas_clicks(
        &mut self,
        response: &egui::Response,
        component_rects: &[egui::Rect],
    ) {
        if !response.clicked() {
            return;
        }

        let Some(click_position) = response.interact_pointer_pos() else {
            return;
        };

        let port_hit_radius = PORT_RADIUS * self.zoom * 2.5;

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
        let component_count = self.circuit.components.len();
        let mut connected = false;

        'outer: for destination_index in 0..component_count {
            let ports = input_port_positions(
                component_rects[destination_index],
                &self.circuit.components[destination_index],
            );

            for (port_position, port_key) in &ports {
                if click_position.distance(*port_position) < port_hit_radius {
                    let source_index = self.wire_state.source_component_index.unwrap();
                    let source_reference =
                        self.build_source_reference(source_index);
                    self.save_undo_checkpoint();

                    let destination_component =
                        &mut self.circuit.components[destination_index];
                    if destination_component.inputs.is_none() {
                        destination_component.inputs = Some(HashMap::new());
                    }
                    destination_component
                        .inputs
                        .as_mut()
                        .unwrap()
                        .insert(port_key.clone(), source_reference.clone());

                    self.wire_state.source_component_index = None;
                    self.status_message = format!(
                        "Connected {source_reference} -> {}.{port_key}",
                        destination_component.name
                    );
                    connected = true;
                    break 'outer;
                }
            }
        }

        if !connected {
            self.wire_state.source_component_index = None;
            self.status_message = "Wire cancelled".into();
        }
    }

    /// Build the correct source reference for a component when creating a wire.
    /// For primitives (clk, nand, dff), the reference is just the component name.
    /// For abstraction instances, it's "instance_name.output_name" (dot-notation).
    fn build_source_reference(&self, source_index: usize) -> String {
        let source_component = &self.circuit.components[source_index];

        if is_primitive(&source_component.component_type) {
            return source_component.name.clone();
        }

        // Look up the abstraction definition to find its output names.
        let abstraction = self
            .circuit
            .abstractions
            .iter()
            .find(|a| a.name == source_component.component_type);

        if let Some(abstraction) = abstraction {
            // Sort output names for deterministic behavior.
            let mut output_names: Vec<&String> = abstraction.outputs.keys().collect();
            output_names.sort();
            if let Some(output_name) = output_names.first() {
                return format!("{}.{}", source_component.name, output_name);
            }
        }

        // Fallback: just use the component name (shouldn't normally happen).
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
        for component_index in (0..component_count).rev() {
            let output_position = output_port_position(component_rects[component_index]);
            if click_position.distance(output_position) < port_hit_radius {
                self.wire_state.source_component_index = Some(component_index);
                self.status_message = format!(
                    "Wiring from '{}' — click an input port",
                    self.circuit.components[component_index].name
                );
                return true;
            }
        }
        false
    }

    /// Handle left-click drag to move components on the canvas.
    fn handle_component_dragging(
        &mut self,
        response: &egui::Response,
        component_rects: &[egui::Rect],
        canvas_origin: egui::Pos2,
        ctx: &egui::Context,
    ) {
        let component_count = self.circuit.components.len();

        // Start a new drag on left mouse button
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pointer_position) = response.interact_pointer_pos() {
                let port_hit_radius = PORT_RADIUS * self.zoom * 2.5;

                // Don't start dragging if the pointer is on a port
                let is_pointer_on_port = (0..component_count).any(|index| {
                    let output_position = output_port_position(component_rects[index]);
                    if pointer_position.distance(output_position) < port_hit_radius {
                        return true;
                    }
                    let ports = input_port_positions(
                        component_rects[index],
                        &self.circuit.components[index],
                    );
                    ports
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

        // Continue dragging — update the component's position with grid snapping
        if let Some(drag_index) = self.dragging_component {
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(pointer_position) = ctx.pointer_interact_pos() {
                    let new_world_position =
                        (pointer_position - self.drag_offset - canvas_origin.to_vec2()) / self.zoom;
                    let snapped_x = snap_to_grid(new_world_position.x);
                    let snapped_y = snap_to_grid(new_world_position.y);
                    self.circuit.components[drag_index].pos = Some([snapped_x, snapped_y]);
                }
            }
            if response.drag_stopped() {
                self.save_undo_checkpoint();
                self.dragging_component = None;
            }
        }
    }
}
