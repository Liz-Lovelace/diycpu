use eframe::egui;
use std::collections::HashMap;

use diycpu::project::*;
use diycpu::types::is_primitive;

use super::app::{App, InteractionMode};
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

    if scaled_grid_spacing <= 4.0 {
        return;
    }

    let offset_x = canvas_offset.x.rem_euclid(scaled_grid_spacing);
    let offset_y = canvas_offset.y.rem_euclid(scaled_grid_spacing);

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

/// Get the simulation value for a wire net's source.
fn wire_net_sim_value(
    net: &WireNet,
    current_step_values: Option<&HashMap<String, bool>>,
) -> Option<bool> {
    let source = net.source.as_ref()?;
    let values = current_step_values?;

    // Try component_id.port_name (for abstraction outputs)
    let dot_key = format!("{}.{}", source.component_id, source.port_name);
    if let Some(&v) = values.get(&dot_key) {
        return Some(v);
    }

    // Fall back to just component_id (for primitives)
    values.get(&source.component_id).copied()
}

/// Draw all wire nets with thick lines and outlines.
fn draw_all_wire_nets(
    painter: &egui::Painter,
    wire_nets: &[WireNet],
    component_ids: &[String],
    component_rects: &[egui::Rect],
    component_types: &[String],
    abstractions: &[ProjectAbstraction],
    current_step_values: Option<&HashMap<String, bool>>,
    is_simulation_active: bool,
    canvas_origin: egui::Pos2,
    zoom: f32,
    editing_abstraction: Option<&ProjectAbstraction>,
    selected_components: &std::collections::HashSet<ComponentId>,
    selected_junctions: &std::collections::HashSet<(WireNetId, JunctionId)>,
) {
    for net in wire_nets {
        let sim_value = wire_net_sim_value(net, current_step_values);
        let wire_color = if is_simulation_active {
            match sim_value {
                Some(true) => WIRE_HIGH,
                Some(false) => WIRE_LOW,
                None => WIRE_UNKNOWN,
            }
        } else {
            // Highlight wires connected to selected components
            let is_highlighted = net
                .source
                .as_ref()
                .map(|s| selected_components.contains(&s.component_id))
                .unwrap_or(false)
                || net
                    .sinks
                    .iter()
                    .any(|s| selected_components.contains(&s.component_id));
            if is_highlighted {
                egui::Color32::from_rgb(255, 255, 100)
            } else {
                WIRE_UNKNOWN
            }
        };

        let outline_color = egui::Color32::from_gray(20);
        let thickness = WIRE_THICKNESS * zoom;
        let outline_thickness = (WIRE_THICKNESS + WIRE_OUTLINE * 2.0) * zoom;

        // Draw segments
        for segment in &net.segments {
            let from_pos = endpoint_screen_pos(
                &segment.from,
                component_ids,
                component_rects,
                component_types,
                abstractions,
                net,
                canvas_origin,
                zoom,
                editing_abstraction,
            );
            let to_pos = endpoint_screen_pos(
                &segment.to,
                component_ids,
                component_rects,
                component_types,
                abstractions,
                net,
                canvas_origin,
                zoom,
                editing_abstraction,
            );

            if let (Some(from), Some(to)) = (from_pos, to_pos) {
                // Draw outline first, then fill
                painter.line_segment(
                    [from, to],
                    egui::Stroke::new(outline_thickness, outline_color),
                );
                painter.line_segment([from, to], egui::Stroke::new(thickness, wire_color));
            }
        }

        // If no segments but there's a source and sinks, draw direct lines
        // (for wire nets that haven't been given visual routing yet)
        if net.segments.is_empty() {
            let source_pos = net.source.as_ref().and_then(|src| {
                let ep = Endpoint::Port {
                    component_id: src.component_id.clone(),
                    port_name: src.port_name.clone(),
                };
                endpoint_screen_pos(
                    &ep,
                    component_ids,
                    component_rects,
                    component_types,
                    abstractions,
                    net,
                    canvas_origin,
                    zoom,
                    editing_abstraction,
                )
            });

            if let Some(src_pos) = source_pos {
                for sink in &net.sinks {
                    let ep = Endpoint::Port {
                        component_id: sink.component_id.clone(),
                        port_name: sink.port_name.clone(),
                    };
                    if let Some(sink_pos) = endpoint_screen_pos(
                        &ep,
                        component_ids,
                        component_rects,
                        component_types,
                        abstractions,
                        net,
                        canvas_origin,
                        zoom,
                        editing_abstraction,
                    ) {
                        // Manhattan routing as fallback
                        let mid_x = (src_pos.x + sink_pos.x) / 2.0;
                        let points = [
                            src_pos,
                            egui::pos2(mid_x, src_pos.y),
                            egui::pos2(mid_x, sink_pos.y),
                            sink_pos,
                        ];
                        for pair in points.windows(2) {
                            painter.line_segment(
                                [pair[0], pair[1]],
                                egui::Stroke::new(outline_thickness, outline_color),
                            );
                            painter.line_segment(
                                [pair[0], pair[1]],
                                egui::Stroke::new(thickness, wire_color),
                            );
                        }
                    }
                }
            }
        }

        // Draw junctions as filled circles
        let junction_radius = JUNCTION_RADIUS * zoom;
        for junction in &net.junctions {
            let pos = canvas_origin + egui::vec2(junction.pos[0], junction.pos[1]) * zoom;
            let is_selected =
                selected_junctions.contains(&(net.id.clone(), junction.id.clone()));
            if is_selected {
                // White selection ring
                painter.circle_filled(pos, junction_radius + 3.0, egui::Color32::WHITE);
            }
            painter.circle_filled(pos, junction_radius + 1.0, outline_color);
            painter.circle_filled(pos, junction_radius, wire_color);
        }
    }
}

/// Draw the in-progress wire from the last endpoint to the current pointer position.
fn draw_wire_in_progress(
    painter: &egui::Painter,
    from_endpoint: &Endpoint,
    wire_nets: &[WireNet],
    component_ids: &[String],
    component_rects: &[egui::Rect],
    component_types: &[String],
    abstractions: &[ProjectAbstraction],
    net_id: &str,
    hover_position: Option<egui::Pos2>,
    canvas_origin: egui::Pos2,
    zoom: f32,
    editing_abstraction: Option<&ProjectAbstraction>,
) {
    let net = wire_nets.iter().find(|n| n.id == net_id);
    let empty_net = WireNet {
        id: String::new(),
        source: None,
        sinks: vec![],
        junctions: vec![],
        segments: vec![],
    };
    let net = net.unwrap_or(&empty_net);

    let from_pos = endpoint_screen_pos(
        from_endpoint,
        component_ids,
        component_rects,
        component_types,
        abstractions,
        net,
        canvas_origin,
        zoom,
        editing_abstraction,
    );

    if let (Some(from), Some(pointer)) = (from_pos, hover_position) {
        painter.line_segment(
            [from, pointer],
            egui::Stroke::new(2.0, PORT_HOVER_COLOR),
        );
        painter.circle_filled(from, PORT_RADIUS * zoom, PORT_HOVER_COLOR);
    }
}

/// Draw a single component: background, border, label, badges, and ports.
fn draw_single_component(
    painter: &egui::Painter,
    component_rect: egui::Rect,
    component: &ProjectComponent,
    abstractions: &[ProjectAbstraction],
    is_selected: bool,
    simulation_value: Option<bool>,
    is_simulation_active: bool,
    hover_position: Option<egui::Pos2>,
    zoom: f32,
) {
    let component_color = color_for_component_type(&component.component_type);
    let label_font_size = (13.0 * zoom).max(7.0);
    let port_font_size = (10.0 * zoom).max(6.0);

    // Background fill
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

    // Border stroke
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

    // Component label
    let label_text = format!("{}\n({})", component.id, component.component_type);
    painter.text(
        component_rect.center(),
        egui::Align2::CENTER_CENTER,
        &label_text,
        egui::FontId::proportional(label_font_size),
        egui::Color32::WHITE,
    );

    // Value badge
    draw_component_badge(
        painter,
        component_rect,
        component,
        simulation_value,
        is_simulation_active,
        zoom,
    );

    // Double-border hint for abstraction instances
    if !is_primitive(&component.component_type) && component.component_type != "clk" {
        let inner_rect = component_rect.shrink(3.0 * zoom);
        painter.rect_stroke(
            inner_rect,
            4.0 * zoom,
            egui::Stroke::new(0.5 * zoom, component_color.linear_multiply(0.4)),
            egui::StrokeKind::Outside,
        );
    }

    // Ports
    draw_component_ports(
        painter,
        component_rect,
        &component.component_type,
        abstractions,
        component_color,
        hover_position,
        zoom,
        port_font_size,
    );
}

fn draw_component_badge(
    painter: &egui::Painter,
    component_rect: egui::Rect,
    component: &ProjectComponent,
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

fn draw_component_ports(
    painter: &egui::Painter,
    component_rect: egui::Rect,
    component_type: &str,
    abstractions: &[ProjectAbstraction],
    component_color: egui::Color32,
    hover_position: Option<egui::Pos2>,
    zoom: f32,
    port_font_size: f32,
) {
    let port_hit_radius = PORT_RADIUS * zoom * 2.5;

    // Input ports (left side)
    for (port_position, port_label) in input_port_positions(component_rect, component_type, abstractions) {
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

    // Output ports (right side)
    for (output_position, output_label) in output_port_positions(component_rect, component_type, abstractions) {
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
            &output_label,
            egui::FontId::proportional(port_font_size),
            egui::Color32::from_gray(200),
        );
    }
}

/// Draw input/output pin labels when editing inside an abstraction.
fn draw_abstraction_pins(
    painter: &egui::Painter,
    abstraction: &ProjectAbstraction,
    canvas_origin: egui::Pos2,
    zoom: f32,
) {
    let font = egui::FontId::proportional((12.0 * zoom).max(8.0));
    let pin_radius = 8.0 * zoom;

    // Input pins
    for (i, input_name) in abstraction.inputs.iter().enumerate() {
        let [wx, wy] = abstraction_input_pin_world_pos(i);
        let pos = canvas_origin + egui::vec2(wx, wy) * zoom;
        painter.circle_filled(pos, pin_radius, INPUT_PIN_COLOR);
        painter.text(
            pos + egui::vec2(pin_radius + 4.0, 0.0),
            egui::Align2::LEFT_CENTER,
            input_name,
            font.clone(),
            INPUT_PIN_COLOR,
        );
    }

    // Output pins
    let mut output_names: Vec<&String> = abstraction.outputs.keys().collect();
    output_names.sort();
    for (i, output_name) in output_names.iter().enumerate() {
        let wy = 40.0 + (i as f32 + 1.0) * 40.0;
        let wx = 500.0; // Fixed position to the right
        let pos = canvas_origin + egui::vec2(wx, wy) * zoom;
        let target = &abstraction.outputs[*output_name];
        let label = format!("{output_name} <- {target}");
        painter.circle_filled(pos, pin_radius, OUTPUT_PIN_COLOR);
        painter.text(
            pos + egui::vec2(-(pin_radius + 4.0), 0.0),
            egui::Align2::RIGHT_CENTER,
            &label,
            font.clone(),
            OUTPUT_PIN_COLOR,
        );
    }
}

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

/// Draw a box selection rectangle.
fn draw_box_selection(
    painter: &egui::Painter,
    start_pos: egui::Pos2,
    current_pos: egui::Pos2,
) {
    let rect = egui::Rect::from_two_pos(start_pos, current_pos);
    painter.rect_filled(
        rect,
        0.0,
        egui::Color32::from_rgba_premultiplied(100, 150, 255, 30),
    );
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255)),
        egui::StrokeKind::Outside,
    );
}

// ── App methods for canvas interaction ──────────────────────────────────────

impl App {
    /// Draw the central canvas with all components, wires, and handle interactions.
    pub fn draw_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let canvas_rect = response.rect;

        // Guard: if editing an abstraction that was deleted, go back
        if let Some(abs_idx) = self.editing_abstraction_index {
            if abs_idx >= self.project.abstractions.len() {
                self.editing_abstraction_index = None;
            }
        }

        // Phase 1: Zoom and pan
        self.handle_scroll_zoom(ctx, canvas_rect);
        self.handle_canvas_panning(&response, ctx);

        // Phase 2: Compute layout
        let canvas_origin = canvas_rect.min + self.canvas_offset;
        let components: Vec<ProjectComponent> = self.active_components().to_vec();
        let abstractions: Vec<ProjectAbstraction> = self.project.abstractions.clone();
        let wire_nets: Vec<WireNet> = self.active_wire_nets().to_vec();
        let component_ids: Vec<String> = components.iter().map(|c| c.id.clone()).collect();
        let component_types: Vec<String> =
            components.iter().map(|c| c.component_type.clone()).collect();
        let component_rects: Vec<egui::Rect> = components
            .iter()
            .map(|c| self.component_screen_rect(c, canvas_origin))
            .collect();
        let hover_position = ctx.input(|i| i.pointer.hover_pos());
        let is_simulation_active = self.simulation_results.is_some()
            && self.editing_abstraction_index.is_none();

        let current_step_values: Option<HashMap<String, bool>> = self
            .simulation_results
            .as_ref()
            .and_then(|results| results.get(self.simulation_step))
            .cloned();

        let editing_abs = self.editing_abstraction().cloned();

        // Phase 3: Draw everything
        draw_grid_background(&painter, canvas_rect, self.canvas_offset, self.zoom);

        // Draw abstraction pins when editing inside one
        if let Some(ref abs) = editing_abs {
            draw_abstraction_pins(&painter, abs, canvas_origin, self.zoom);
        }

        draw_all_wire_nets(
            &painter,
            &wire_nets,
            &component_ids,
            &component_rects,
            &component_types,
            &abstractions,
            current_step_values.as_ref(),
            is_simulation_active,
            canvas_origin,
            self.zoom,
            editing_abs.as_ref(),
            &self.selected_components,
            &self.selected_junctions,
        );

        // Draw wire in progress
        if let InteractionMode::DrawingWire {
            ref net_id,
            ref from_endpoint,
        } = self.interaction_mode
        {
            draw_wire_in_progress(
                &painter,
                from_endpoint,
                &wire_nets,
                &component_ids,
                &component_rects,
                &component_types,
                &abstractions,
                net_id,
                hover_position,
                canvas_origin,
                self.zoom,
                editing_abs.as_ref(),
            );
        }

        // Draw box selection rectangle
        if let InteractionMode::BoxSelecting { start_pos } = self.interaction_mode {
            if let Some(current_pos) = hover_position {
                draw_box_selection(&painter, start_pos, current_pos);
            }
        }

        for (i, component) in components.iter().enumerate() {
            let sim_value = current_step_values
                .as_ref()
                .and_then(|values| values.get(&component.id).copied());
            draw_single_component(
                &painter,
                component_rects[i],
                component,
                &abstractions,
                self.selected_components.contains(&component.id),
                sim_value,
                is_simulation_active,
                hover_position,
                self.zoom,
            );
        }

        draw_zoom_indicator(&painter, canvas_rect, self.zoom);

        // Tooltip on hover
        if let Some(hover_pos) = hover_position {
            if matches!(self.interaction_mode, InteractionMode::Normal) {
                if let Some(idx) = find_component_at_position(hover_pos, &component_rects) {
                    let comp = &components[idx];
                    let mut tip = format!("{} ({})", comp.id, comp.component_type);
                    // Show connections
                    let connections = self.connections_for_component(&comp.id);
                    for (port, source) in &connections {
                        tip.push_str(&format!("\n  {port} <- {source}"));
                    }
                    if let Some(ref vals) = current_step_values {
                        if let Some(&v) = vals.get(&comp.id) {
                            tip.push_str(&format!("\n  = {}", if v { "1" } else { "0" }));
                        }
                    }
                    response.clone().on_hover_text_at_pointer(&tip);
                }
            }
        }

        // Phase 4: Interaction
        self.handle_canvas_clicks(
            &response,
            &components,
            &component_rects,
            &component_ids,
            &component_types,
            &abstractions,
            canvas_origin,
            ctx,
        );
        self.handle_dragging(
            &response,
            &components,
            &component_rects,
            &component_ids,
            &component_types,
            &abstractions,
            canvas_origin,
            ctx,
        );
    }

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

    fn handle_canvas_panning(&mut self, response: &egui::Response, ctx: &egui::Context) {
        // Don't start panning while drawing a wire
        if matches!(self.interaction_mode, InteractionMode::DrawingWire { .. }) {
            return;
        }

        if let Some(pointer_position) = response.interact_pointer_pos() {
            if response.drag_started_by(egui::PointerButton::Middle)
                || response.drag_started_by(egui::PointerButton::Secondary)
            {
                self.is_panning = true;
                self.last_pan_pointer_position = Some(pointer_position);
            }
        }

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

    fn handle_canvas_clicks(
        &mut self,
        response: &egui::Response,
        components: &[ProjectComponent],
        component_rects: &[egui::Rect],
        component_ids: &[String],
        component_types: &[String],
        abstractions: &[ProjectAbstraction],
        canvas_origin: egui::Pos2,
        ctx: &egui::Context,
    ) {
        let port_hit_radius = PORT_RADIUS * self.zoom * 2.5;

        // Handle right-click: cancel wire drawing or delete junctions
        if response.secondary_clicked() {
            if matches!(self.interaction_mode, InteractionMode::DrawingWire { .. }) {
                self.cancel_wire_drawing();
                return;
            }
            if let Some(click_pos) = response.interact_pointer_pos() {
                self.handle_junction_delete(click_pos, canvas_origin);
            }
        }

        // Handle double-click: enter abstraction
        let double_clicked =
            ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
        if double_clicked {
            if let Some(click_pos) = ctx.input(|i| i.pointer.interact_pos()) {
                if matches!(self.interaction_mode, InteractionMode::Normal) {
                    if let Some(comp_idx) =
                        find_component_at_position(click_pos, component_rects)
                    {
                        let comp_type = &components[comp_idx].component_type;
                        if !is_primitive(comp_type) {
                            if let Some(abs_idx) = self
                                .project
                                .abstractions
                                .iter()
                                .position(|a| a.name == *comp_type)
                            {
                                self.editing_abstraction_index = Some(abs_idx);
                                self.clear_selection();
                                self.canvas_offset = egui::Vec2::ZERO;
                                self.status_message =
                                    format!("Editing abstraction: {comp_type}");
                                return;
                            }
                        }
                    }
                }

                // Double-click on a wire segment to insert a junction
                if matches!(self.interaction_mode, InteractionMode::Normal) {
                    self.handle_segment_split(click_pos, component_ids, component_rects, component_types, abstractions, canvas_origin);
                }
            }
        }

        if !response.clicked() {
            return;
        }

        let Some(click_position) = response.interact_pointer_pos() else {
            return;
        };

        match self.interaction_mode.clone() {
            InteractionMode::DrawingWire { net_id, from_endpoint } => {
                // Check if clicking an input port to complete the wire
                let mut connected = false;
                for (comp_idx, comp) in components.iter().enumerate() {
                    let ports = input_port_positions(
                        component_rects[comp_idx],
                        &comp.component_type,
                        abstractions,
                    );
                    for (port_pos, port_name) in &ports {
                        if click_position.distance(*port_pos) < port_hit_radius {
                            self.save_undo_checkpoint();

                            // Add sink to the wire net
                            let nets = self.active_wire_nets_mut();
                            if let Some(net) = nets.iter_mut().find(|n| n.id == net_id) {
                                net.sinks.push(PortRef {
                                    component_id: comp.id.clone(),
                                    port_name: port_name.clone(),
                                });
                                // Add segment from last endpoint to this port
                                net.segments.push(Segment {
                                    from: from_endpoint.clone(),
                                    to: Endpoint::Port {
                                        component_id: comp.id.clone(),
                                        port_name: port_name.clone(),
                                    },
                                });
                            }

                            self.interaction_mode = InteractionMode::Normal;
                            self.status_message = format!(
                                "Connected to {}.{port_name}",
                                comp.id,
                            );
                            connected = true;
                            break;
                        }
                    }
                    if connected {
                        break;
                    }
                }

                if !connected {
                    // Check if clicking an existing junction in the same net
                    let hit_radius = JUNCTION_RADIUS * self.zoom * 2.0;
                    let wire_nets = self.active_wire_nets().to_vec();
                    let mut connected_to_junction = false;

                    if let Some(net) = wire_nets.iter().find(|n| n.id == net_id) {
                        for junction in &net.junctions {
                            let screen_pos = canvas_origin
                                + egui::vec2(junction.pos[0], junction.pos[1]) * self.zoom;
                            if click_position.distance(screen_pos) < hit_radius {
                                // Guard against self-loops
                                let is_self = matches!(
                                    &from_endpoint,
                                    Endpoint::Junction { junction_id } if *junction_id == junction.id
                                );
                                if !is_self {
                                    self.save_undo_checkpoint();
                                    let target_junction_id = junction.id.clone();
                                    let nets = self.active_wire_nets_mut();
                                    if let Some(net) =
                                        nets.iter_mut().find(|n| n.id == net_id)
                                    {
                                        net.segments.push(Segment {
                                            from: from_endpoint.clone(),
                                            to: Endpoint::Junction {
                                                junction_id: target_junction_id,
                                            },
                                        });
                                    }
                                    self.interaction_mode = InteractionMode::Normal;
                                    self.status_message = "Connected to junction".into();
                                    connected_to_junction = true;
                                }
                                break;
                            }
                        }
                    }

                    if !connected_to_junction {
                        // Click on empty space while drawing → place junction
                        let world_pos =
                            (click_position - canvas_origin.to_vec2()) / self.zoom;
                        let snapped_x = snap_to_grid(world_pos.x);
                        let snapped_y = snap_to_grid(world_pos.y);
                        let junction_id = self.generate_junction_id();

                        let nets = self.active_wire_nets_mut();
                        if let Some(net) = nets.iter_mut().find(|n| n.id == net_id) {
                            net.junctions.push(Junction {
                                id: junction_id.clone(),
                                pos: [snapped_x, snapped_y],
                            });
                            net.segments.push(Segment {
                                from: from_endpoint,
                                to: Endpoint::Junction {
                                    junction_id: junction_id.clone(),
                                },
                            });
                        }

                        self.interaction_mode = InteractionMode::DrawingWire {
                            net_id,
                            from_endpoint: Endpoint::Junction { junction_id },
                        };
                        self.status_message =
                            "Junction placed — continue wiring or click port".into();
                    }
                }
            }
            InteractionMode::Normal => {
                // Check if clicking an output port to start wiring
                let started_wire = self.try_start_wire_from_output_port(
                    click_position,
                    components,
                    component_rects,
                    abstractions,
                    port_hit_radius,
                    canvas_origin,
                );

                if !started_wire {
                    let alt_held = ctx.input(|i| i.modifiers.alt);
                    let ctrl_held = ctx.input(|i| i.modifiers.ctrl || i.modifiers.mac_cmd);

                    // Check if clicking a junction
                    if let Some((net_id, junction_id)) =
                        self.find_junction_at(click_position, canvas_origin)
                    {
                        if alt_held {
                            // Alt+click junction → start branching wire
                            self.interaction_mode = InteractionMode::DrawingWire {
                                net_id,
                                from_endpoint: Endpoint::Junction { junction_id },
                            };
                            self.status_message =
                                "Branching from junction — click port/junction/empty"
                                    .into();
                        } else {
                            // Normal/Ctrl click → select/deselect junction
                            let key = (net_id, junction_id);
                            if ctrl_held {
                                if self.selected_junctions.contains(&key) {
                                    self.selected_junctions.remove(&key);
                                } else {
                                    self.selected_junctions.insert(key);
                                }
                            } else {
                                self.clear_selection();
                                self.selected_junctions.insert(key);
                            }
                        }
                    } else {
                        // Normal click: select/deselect component
                        if let Some(comp_idx) =
                            find_component_at_position(click_position, component_rects)
                        {
                            let comp_id = &components[comp_idx].id;
                            if ctrl_held {
                                // Toggle selection
                                if self.selected_components.contains(comp_id) {
                                    self.selected_components.remove(comp_id);
                                } else {
                                    self.selected_components.insert(comp_id.clone());
                                }
                            } else {
                                // Replace selection
                                self.clear_selection();
                                self.selected_components.insert(comp_id.clone());
                            }
                        } else if !ctrl_held {
                            self.clear_selection();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Try to start wire drawing from an output port. Returns true if started.
    fn try_start_wire_from_output_port(
        &mut self,
        click_position: egui::Pos2,
        components: &[ProjectComponent],
        component_rects: &[egui::Rect],
        abstractions: &[ProjectAbstraction],
        port_hit_radius: f32,
        _canvas_origin: egui::Pos2,
    ) -> bool {
        for comp_idx in (0..components.len()).rev() {
            let out_ports = output_port_positions(
                component_rects[comp_idx],
                &components[comp_idx].component_type,
                abstractions,
            );
            for (port_pos, port_name) in &out_ports {
                if click_position.distance(*port_pos) < port_hit_radius {
                    // Create a new wire net with this source
                    let net_id = self.generate_wire_net_id();
                    let source_port = PortRef {
                        component_id: components[comp_idx].id.clone(),
                        port_name: port_name.clone(),
                    };
                    let from_endpoint = Endpoint::Port {
                        component_id: components[comp_idx].id.clone(),
                        port_name: port_name.clone(),
                    };
                    self.active_wire_nets_mut().push(WireNet {
                        id: net_id.clone(),
                        source: Some(source_port),
                        sinks: vec![],
                        junctions: vec![],
                        segments: vec![],
                    });
                    self.interaction_mode = InteractionMode::DrawingWire {
                        net_id,
                        from_endpoint,
                    };
                    self.status_message = format!(
                        "Wiring from '{}'.{port_name} — click an input port",
                        components[comp_idx].id,
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Find a junction at the given screen position. Returns (net_id, junction_id) if found.
    fn find_junction_at(
        &self,
        screen_position: egui::Pos2,
        canvas_origin: egui::Pos2,
    ) -> Option<(WireNetId, JunctionId)> {
        let hit_radius = JUNCTION_RADIUS * self.zoom * 2.0;
        for net in self.active_wire_nets() {
            for junction in &net.junctions {
                let pos =
                    canvas_origin + egui::vec2(junction.pos[0], junction.pos[1]) * self.zoom;
                if screen_position.distance(pos) < hit_radius {
                    return Some((net.id.clone(), junction.id.clone()));
                }
            }
        }
        None
    }

    /// Handle right-click to delete a junction.
    fn handle_junction_delete(&mut self, click_position: egui::Pos2, canvas_origin: egui::Pos2) {
        let hit_radius = JUNCTION_RADIUS * self.zoom * 2.0;
        let wire_nets = self.active_wire_nets().to_vec();

        for net in &wire_nets {
            for junction in &net.junctions {
                let screen_pos =
                    canvas_origin + egui::vec2(junction.pos[0], junction.pos[1]) * self.zoom;
                if click_position.distance(screen_pos) < hit_radius {
                    self.save_undo_checkpoint();
                    let junction_id = junction.id.clone();
                    let net_id = net.id.clone();

                    let nets = self.active_wire_nets_mut();
                    if let Some(net) = nets.iter_mut().find(|n| n.id == net_id) {
                        // Remove segments connected to this junction
                        net.segments.retain(|seg| {
                            !matches!(&seg.from, Endpoint::Junction { junction_id: jid } if jid == &junction_id)
                                && !matches!(&seg.to, Endpoint::Junction { junction_id: jid } if jid == &junction_id)
                        });
                        // Remove the junction itself
                        net.junctions.retain(|j| j.id != junction_id);
                    }

                    self.status_message = "Junction deleted".into();
                    return;
                }
            }
        }
    }

    /// Handle double-click on a wire segment to insert a junction.
    fn handle_segment_split(
        &mut self,
        click_position: egui::Pos2,
        component_ids: &[String],
        component_rects: &[egui::Rect],
        component_types: &[String],
        abstractions: &[ProjectAbstraction],
        canvas_origin: egui::Pos2,
    ) {
        let hit_distance = 10.0 * self.zoom;
        let editing_abs = self.editing_abstraction().cloned();
        let wire_nets = self.active_wire_nets().to_vec();

        for net in &wire_nets {
            for (seg_idx, segment) in net.segments.iter().enumerate() {
                let from_pos = endpoint_screen_pos(
                    &segment.from,
                    component_ids,
                    component_rects,
                    component_types,
                    abstractions,
                    net,
                    canvas_origin,
                    self.zoom,
                    editing_abs.as_ref(),
                );
                let to_pos = endpoint_screen_pos(
                    &segment.to,
                    component_ids,
                    component_rects,
                    component_types,
                    abstractions,
                    net,
                    canvas_origin,
                    self.zoom,
                    editing_abs.as_ref(),
                );

                if let (Some(from), Some(to)) = (from_pos, to_pos) {
                    let dist = point_to_segment_distance(click_position, from, to);
                    if dist < hit_distance {
                        self.save_undo_checkpoint();
                        let world_pos =
                            (click_position - canvas_origin.to_vec2()) / self.zoom;
                        let snapped_x = snap_to_grid(world_pos.x);
                        let snapped_y = snap_to_grid(world_pos.y);
                        let junction_id = self.generate_junction_id();

                        let nets = self.active_wire_nets_mut();
                        if let Some(net) = nets.iter_mut().find(|n| n.id == net.id) {
                            let old_from = net.segments[seg_idx].from.clone();
                            let old_to = net.segments[seg_idx].to.clone();

                            // Add the new junction
                            net.junctions.push(Junction {
                                id: junction_id.clone(),
                                pos: [snapped_x, snapped_y],
                            });

                            // Replace old segment with two new ones
                            net.segments[seg_idx] = Segment {
                                from: old_from,
                                to: Endpoint::Junction {
                                    junction_id: junction_id.clone(),
                                },
                            };
                            net.segments.insert(
                                seg_idx + 1,
                                Segment {
                                    from: Endpoint::Junction { junction_id },
                                    to: old_to,
                                },
                            );
                        }

                        self.status_message = "Junction inserted".into();
                        return;
                    }
                }
            }
        }
    }

    /// Handle left-click drag for component/junction dragging and box selection.
    fn handle_dragging(
        &mut self,
        response: &egui::Response,
        components: &[ProjectComponent],
        component_rects: &[egui::Rect],
        component_ids: &[String],
        _component_types: &[String],
        abstractions: &[ProjectAbstraction],
        canvas_origin: egui::Pos2,
        ctx: &egui::Context,
    ) {
        let port_hit_radius = PORT_RADIUS * self.zoom * 2.5;

        // Start a new drag
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pointer_position) = response.interact_pointer_pos() {
                // Skip if we're in DrawingWire mode
                if matches!(self.interaction_mode, InteractionMode::DrawingWire { .. }) {
                    return;
                }

                // Check if dragging from a connected port (Bug 4: detach wire)
                if let Some(detach_result) = self.try_detach_wire_from_port(
                    pointer_position,
                    components,
                    component_rects,
                    abstractions,
                    port_hit_radius,
                    canvas_origin,
                ) {
                    self.save_undo_checkpoint();
                    self.interaction_mode = InteractionMode::DraggingJunction {
                        net_id: detach_result.0,
                        junction_id: detach_result.1,
                    };
                    return;
                }

                // Check if starting a junction drag
                if self.try_start_junction_drag(pointer_position, canvas_origin) {
                    return;
                }

                // Don't start dragging if on an unconnected port (output ports start wiring)
                let is_on_port = (0..components.len()).any(|idx| {
                    let out_ports = output_port_positions(
                        component_rects[idx],
                        &components[idx].component_type,
                        abstractions,
                    );
                    if out_ports
                        .iter()
                        .any(|(pos, _)| pointer_position.distance(*pos) < port_hit_radius)
                    {
                        return true;
                    }
                    let in_ports = input_port_positions(
                        component_rects[idx],
                        &components[idx].component_type,
                        abstractions,
                    );
                    in_ports
                        .iter()
                        .any(|(pos, _)| pointer_position.distance(*pos) < port_hit_radius)
                });

                if !is_on_port {
                    if let Some(comp_idx) =
                        find_component_at_position(pointer_position, component_rects)
                    {
                        // Start dragging components
                        let comp_id = &components[comp_idx].id;

                        // If the clicked component isn't selected, make it the sole selection
                        if !self.selected_components.contains(comp_id) {
                            self.clear_selection();
                            self.selected_components.insert(comp_id.clone());
                        }

                        // Record original positions of all selected components
                        let world_pos =
                            (pointer_position - canvas_origin.to_vec2()) / self.zoom;
                        let mut original_positions = HashMap::new();
                        for sel_id in &self.selected_components {
                            if let Some(c) = components.iter().find(|c| c.id == *sel_id) {
                                original_positions.insert(sel_id.clone(), c.pos);
                            }
                        }

                        // Also record original positions of selected junctions
                        let mut original_junction_positions = HashMap::new();
                        let wire_nets = self.active_wire_nets().to_vec();
                        for (jnet_id, jid) in &self.selected_junctions {
                            if let Some(net) = wire_nets.iter().find(|n| n.id == *jnet_id) {
                                if let Some(j) = net.junctions.iter().find(|j| j.id == *jid)
                                {
                                    original_junction_positions
                                        .insert((jnet_id.clone(), jid.clone()), j.pos);
                                }
                            }
                        }

                        self.interaction_mode = InteractionMode::DraggingComponents {
                            original_positions,
                            original_junction_positions,
                            drag_start_world: [world_pos.x, world_pos.y],
                        };
                    } else {
                        // Start box selection
                        self.interaction_mode = InteractionMode::BoxSelecting {
                            start_pos: pointer_position,
                        };
                    }
                }
            }
        }

        // Continue drag
        match self.interaction_mode.clone() {
            InteractionMode::DraggingJunction {
                ref net_id,
                ref junction_id,
            } => {
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(pointer_position) = ctx.pointer_interact_pos() {
                        let world_pos =
                            (pointer_position - canvas_origin.to_vec2()) / self.zoom;
                        let snapped_x = snap_to_grid(world_pos.x);
                        let snapped_y = snap_to_grid(world_pos.y);

                        let net_id = net_id.clone();
                        let junction_id = junction_id.clone();
                        for net in self.active_wire_nets_mut() {
                            if net.id == net_id {
                                if let Some(j) =
                                    net.junctions.iter_mut().find(|j| j.id == junction_id)
                                {
                                    j.pos = [snapped_x, snapped_y];
                                }
                                break;
                            }
                        }
                    }
                }
                if response.drag_stopped() {
                    // Snap-to-port: check if junction is being dropped near a port
                    if let Some(drop_pos) = ctx.pointer_interact_pos() {
                        let net_id = net_id.clone();
                        let junction_id = junction_id.clone();
                        self.try_snap_junction_to_port(
                            drop_pos,
                            &net_id,
                            &junction_id,
                            components,
                            component_rects,
                            abstractions,
                            port_hit_radius,
                        );
                    }
                    self.save_undo_checkpoint();
                    self.interaction_mode = InteractionMode::Normal;
                }
            }
            InteractionMode::DraggingComponents {
                ref original_positions,
                ref original_junction_positions,
                ref drag_start_world,
            } => {
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(pointer_position) = ctx.pointer_interact_pos() {
                        let current_world =
                            (pointer_position - canvas_origin.to_vec2()) / self.zoom;
                        let dx = current_world.x - drag_start_world[0];
                        let dy = current_world.y - drag_start_world[1];

                        let original_positions = original_positions.clone();
                        for (comp_id, [ox, oy]) in &original_positions {
                            if let Some(c) = self
                                .active_components_mut()
                                .iter_mut()
                                .find(|c| c.id == *comp_id)
                            {
                                c.pos = [snap_to_grid(ox + dx), snap_to_grid(oy + dy)];
                            }
                        }

                        // Also move selected junctions
                        let original_junction_positions = original_junction_positions.clone();
                        for ((jnet_id, jid), [ox, oy]) in &original_junction_positions {
                            for net in self.active_wire_nets_mut() {
                                if net.id == *jnet_id {
                                    if let Some(j) =
                                        net.junctions.iter_mut().find(|j| j.id == *jid)
                                    {
                                        j.pos =
                                            [snap_to_grid(ox + dx), snap_to_grid(oy + dy)];
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
                if response.drag_stopped() {
                    self.save_undo_checkpoint();
                    self.interaction_mode = InteractionMode::Normal;
                }
            }
            InteractionMode::BoxSelecting { start_pos } => {
                if response.drag_stopped() {
                    // Finalize box selection: select all components and junctions in the rectangle
                    if let Some(end_pos) = ctx.pointer_interact_pos() {
                        let selection_rect = egui::Rect::from_two_pos(start_pos, end_pos);
                        self.clear_selection();

                        // Select components
                        for (idx, rect) in component_rects.iter().enumerate() {
                            if selection_rect.intersects(*rect) {
                                self.selected_components
                                    .insert(component_ids[idx].clone());
                            }
                        }

                        // Select junctions
                        let wire_nets = self.active_wire_nets().to_vec();
                        for net in &wire_nets {
                            for junction in &net.junctions {
                                let screen_pos = canvas_origin
                                    + egui::vec2(junction.pos[0], junction.pos[1])
                                        * self.zoom;
                                if selection_rect.contains(screen_pos) {
                                    self.selected_junctions.insert((
                                        net.id.clone(),
                                        junction.id.clone(),
                                    ));
                                }
                            }
                        }

                        let total =
                            self.selected_components.len() + self.selected_junctions.len();
                        if total > 0 {
                            self.status_message = format!("{total} items selected");
                        }
                    }
                    self.interaction_mode = InteractionMode::Normal;
                }
            }
            _ => {}
        }
    }

    /// Find a port at the given screen position. Returns (component_id, port_name, is_output).
    fn find_port_at_position(
        &self,
        screen_position: egui::Pos2,
        components: &[ProjectComponent],
        component_rects: &[egui::Rect],
        abstractions: &[ProjectAbstraction],
        hit_radius: f32,
    ) -> Option<(String, String, bool)> {
        for (idx, comp) in components.iter().enumerate() {
            // Check output ports
            for (pos, name) in
                output_port_positions(component_rects[idx], &comp.component_type, abstractions)
            {
                if screen_position.distance(pos) < hit_radius {
                    return Some((comp.id.clone(), name, true));
                }
            }
            // Check input ports
            for (pos, name) in
                input_port_positions(component_rects[idx], &comp.component_type, abstractions)
            {
                if screen_position.distance(pos) < hit_radius {
                    return Some((comp.id.clone(), name, false));
                }
            }
        }
        None
    }

    /// Try to detach a wire from a connected port, creating a draggable junction.
    /// Returns (net_id, junction_id) if detachment occurred.
    fn try_detach_wire_from_port(
        &mut self,
        pointer_position: egui::Pos2,
        components: &[ProjectComponent],
        component_rects: &[egui::Rect],
        abstractions: &[ProjectAbstraction],
        hit_radius: f32,
        canvas_origin: egui::Pos2,
    ) -> Option<(WireNetId, JunctionId)> {
        let (comp_id, port_name, is_output) = self.find_port_at_position(
            pointer_position,
            components,
            component_rects,
            abstractions,
            hit_radius,
        )?;

        // Find the wire net connected to this port
        let wire_nets = self.active_wire_nets().to_vec();
        let mut found_net_id = None;

        if is_output {
            // Find net where source matches this port
            for net in &wire_nets {
                if let Some(ref source) = net.source {
                    if source.component_id == comp_id && source.port_name == port_name {
                        found_net_id = Some(net.id.clone());
                        break;
                    }
                }
            }
        } else {
            // Find net where a sink matches this port
            for net in &wire_nets {
                if net
                    .sinks
                    .iter()
                    .any(|s| s.component_id == comp_id && s.port_name == port_name)
                {
                    found_net_id = Some(net.id.clone());
                    break;
                }
            }
        }

        let net_id = found_net_id?;

        // Compute world position for the new junction (at the port position)
        let world_pos = (pointer_position - canvas_origin.to_vec2()) / self.zoom;
        let junction_id = self.generate_junction_id();

        let port_endpoint = Endpoint::Port {
            component_id: comp_id.clone(),
            port_name: port_name.clone(),
        };
        let junction_endpoint = Endpoint::Junction {
            junction_id: junction_id.clone(),
        };

        // Modify the net
        for net in self.active_wire_nets_mut() {
            if net.id == net_id {
                // Add the junction
                net.junctions.push(Junction {
                    id: junction_id.clone(),
                    pos: [snap_to_grid(world_pos.x), snap_to_grid(world_pos.y)],
                });

                // Replace all segment endpoints referencing this port with the junction
                for seg in &mut net.segments {
                    if seg.from == port_endpoint {
                        seg.from = junction_endpoint.clone();
                    }
                    if seg.to == port_endpoint {
                        seg.to = junction_endpoint.clone();
                    }
                }

                // Remove logical connection
                if is_output {
                    net.source = None;
                } else {
                    net.sinks
                        .retain(|s| !(s.component_id == comp_id && s.port_name == port_name));
                }

                break;
            }
        }

        self.status_message = "Detached wire — drag to reconnect".into();
        Some((net_id, junction_id))
    }

    /// When a junction is dropped near a port, reconnect the wire to that port.
    fn try_snap_junction_to_port(
        &mut self,
        drop_position: egui::Pos2,
        net_id: &str,
        junction_id: &str,
        components: &[ProjectComponent],
        component_rects: &[egui::Rect],
        abstractions: &[ProjectAbstraction],
        hit_radius: f32,
    ) {
        let Some((comp_id, port_name, is_output)) = self.find_port_at_position(
            drop_position,
            components,
            component_rects,
            abstractions,
            hit_radius,
        ) else {
            return;
        };

        let port_endpoint = Endpoint::Port {
            component_id: comp_id.clone(),
            port_name: port_name.clone(),
        };
        let junction_endpoint = Endpoint::Junction {
            junction_id: junction_id.to_string(),
        };

        for net in self.active_wire_nets_mut() {
            if net.id == net_id {
                // Replace junction references in segments with the port
                for seg in &mut net.segments {
                    if seg.from == junction_endpoint {
                        seg.from = port_endpoint.clone();
                    }
                    if seg.to == junction_endpoint {
                        seg.to = port_endpoint.clone();
                    }
                }

                // Remove the junction
                net.junctions.retain(|j| j.id != junction_id);

                // Add logical connection
                if is_output {
                    net.source = Some(PortRef {
                        component_id: comp_id.clone(),
                        port_name: port_name.clone(),
                    });
                } else {
                    // Only add if not already a sink
                    if !net
                        .sinks
                        .iter()
                        .any(|s| s.component_id == comp_id && s.port_name == port_name)
                    {
                        net.sinks.push(PortRef {
                            component_id: comp_id,
                            port_name,
                        });
                    }
                }

                break;
            }
        }

        self.status_message = "Wire reconnected".into();
    }

    /// Try to start dragging a junction. Returns true if a junction was found.
    fn try_start_junction_drag(
        &mut self,
        pointer_position: egui::Pos2,
        canvas_origin: egui::Pos2,
    ) -> bool {
        let hit_radius = JUNCTION_RADIUS * self.zoom * 2.0;
        let wire_nets = self.active_wire_nets().to_vec();

        for net in &wire_nets {
            for junction in &net.junctions {
                let screen_pos =
                    canvas_origin + egui::vec2(junction.pos[0], junction.pos[1]) * self.zoom;
                if pointer_position.distance(screen_pos) < hit_radius {
                    self.interaction_mode = InteractionMode::DraggingJunction {
                        net_id: net.id.clone(),
                        junction_id: junction.id.clone(),
                    };
                    return true;
                }
            }
        }
        false
    }
}
