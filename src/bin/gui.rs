use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;

use diycpu::compiler::compile;
use diycpu::expander::expand;
use diycpu::simulator::{format_table, simulate};
use diycpu::types::{CircuitDef, ComponentDef};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "diycpu editor",
        options,
        Box::new(|_cc| Ok(Box::new(App::default()))),
    )
}

// ── Colours ──────────────────────────────────────────────────────────────────

const CLK_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 180, 255);
const NAND_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 80);
const DFF_COLOR: egui::Color32 = egui::Color32::from_rgb(130, 220, 130);
const ABS_COLOR: egui::Color32 = egui::Color32::from_rgb(200, 160, 255);

fn type_color(t: &str) -> egui::Color32 {
    match t {
        "clk" => CLK_COLOR,
        "nand" => NAND_COLOR,
        "dff" => DFF_COLOR,
        _ => ABS_COLOR,
    }
}

// ── Constants ────────────────────────────────────────────────────────────────

const COMP_W: f32 = 130.0;
const COMP_H: f32 = 60.0;
const GRID: f32 = 20.0;
const PORT_R: f32 = 4.0;

fn snap(v: f32) -> f32 {
    (v / GRID).round() * GRID
}

// ── App state ────────────────────────────────────────────────────────────────

struct App {
    circuit: CircuitDef,
    file_path: Option<PathBuf>,
    selected: Option<usize>,
    dragging: Option<usize>,
    drag_offset: egui::Vec2,
    canvas_offset: egui::Vec2,
    canvas_drag: bool,
    last_canvas_pointer: Option<egui::Pos2>,

    // Add-component dialog
    add_type: String,
    add_name: String,

    // Simulation results
    sim_output: Option<String>,
    sim_error: Option<String>,

    // Status bar message
    status: String,
}

impl Default for App {
    fn default() -> Self {
        Self {
            circuit: CircuitDef {
                name: "untitled".into(),
                steps: 8,
                abstractions: vec![],
                components: vec![],
            },
            file_path: None,
            selected: None,
            dragging: None,
            drag_offset: egui::Vec2::ZERO,
            canvas_offset: egui::Vec2::ZERO,
            canvas_drag: false,
            last_canvas_pointer: None,
            add_type: "nand".into(),
            add_name: String::new(),
            sim_output: None,
            sim_error: None,
            status: "Ready".into(),
        }
    }
}

impl App {
    /// Position for component i (uses stored pos or auto-layout).
    fn comp_pos(&self, i: usize) -> egui::Pos2 {
        let c = &self.circuit.components[i];
        if let Some([x, y]) = c.pos {
            egui::pos2(x, y)
        } else {
            // Auto-layout: grid with some spacing
            let cols = 4;
            let col = i % cols;
            let row = i / cols;
            egui::pos2(
                80.0 + col as f32 * (COMP_W + 40.0),
                80.0 + row as f32 * (COMP_H + 60.0),
            )
        }
    }

    /// Find component index by name.
    fn find_comp(&self, name: &str) -> Option<usize> {
        self.circuit
            .components
            .iter()
            .position(|c| c.name == name)
    }

    /// Run simulation.
    fn run_sim(&mut self) {
        self.sim_output = None;
        self.sim_error = None;

        let expanded = match expand(&self.circuit) {
            Ok(e) => e,
            Err(e) => {
                self.sim_error = Some(format!("Expand error: {e}"));
                return;
            }
        };

        let compiled = match compile(&expanded.def) {
            Ok(c) => c,
            Err(e) => {
                self.sim_error = Some(format!("Compile error: {e}"));
                return;
            }
        };

        let mut results = simulate(&compiled);
        for step in &mut results {
            for (alias, target) in &expanded.aliases {
                if let Some(&val) = step.get(target) {
                    step.insert(alias.clone(), val);
                }
            }
        }

        self.sim_output = Some(format!(
            "=== {} ===\n{}",
            compiled.name,
            format_table(&compiled, &results)
        ));
        self.status = "Simulation complete".into();
    }

    fn save_yaml(&mut self) {
        if let Some(path) = &self.file_path {
            match serde_yaml::to_string(&self.circuit) {
                Ok(yaml) => match std::fs::write(path, &yaml) {
                    Ok(_) => self.status = format!("Saved to {}", path.display()),
                    Err(e) => self.status = format!("Save error: {e}"),
                },
                Err(e) => self.status = format!("Serialize error: {e}"),
            }
        } else {
            self.status = "No file path set — use Save As".into();
        }
    }

    fn load_yaml(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_yaml::from_str::<CircuitDef>(&contents) {
                Ok(def) => {
                    self.circuit = def;
                    self.file_path = Some(path.clone());
                    self.selected = None;
                    self.sim_output = None;
                    self.sim_error = None;
                    self.status = format!("Loaded {}", path.display());
                }
                Err(e) => self.status = format!("YAML parse error: {e}"),
            },
            Err(e) => self.status = format!("Read error: {e}"),
        }
    }
}

// ── egui rendering ───────────────────────────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Top menu bar ─────────────────────────────────────────────────
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        if let Some(path) = rfd_open() {
                            self.load_yaml(path);
                        }
                        ui.close();
                    }
                    if ui.button("Save").clicked() {
                        self.save_yaml();
                        ui.close();
                    }
                    if ui.button("Save As…").clicked() {
                        if let Some(path) = rfd_save() {
                            self.file_path = Some(path);
                            self.save_yaml();
                        }
                        ui.close();
                    }
                });
                ui.separator();
                if ui.button("▶ Simulate").clicked() {
                    self.run_sim();
                }
                ui.separator();
                ui.label(&self.status);
            });
        });

        // ── Right panel: properties & add component ──────────────────────
        egui::SidePanel::right("props")
            .min_width(280.0)
            .show(ctx, |ui| {
                ui.heading("Circuit");
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.circuit.name);
                });
                ui.horizontal(|ui| {
                    ui.label("Steps:");
                    ui.add(egui::DragValue::new(&mut self.circuit.steps).range(1..=10000));
                });

                ui.separator();
                ui.heading("Add Component");
                ui.horizontal(|ui| {
                    ui.label("Type:");
                    egui::ComboBox::from_id_salt("add_type")
                        .selected_text(&self.add_type)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.add_type, "clk".into(), "clk");
                            ui.selectable_value(&mut self.add_type, "nand".into(), "nand");
                            ui.selectable_value(&mut self.add_type, "dff".into(), "dff");
                            // Show abstraction types too
                            for abs in &self.circuit.abstractions {
                                let n = abs.name.clone();
                                ui.selectable_value(&mut self.add_type, n.clone(), &n);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut self.add_name);
                });
                if ui.button("Add").clicked() && !self.add_name.is_empty() {
                    let mut inputs = None;
                    if self.add_type == "nand" {
                        let mut m = HashMap::new();
                        m.insert("a".into(), String::new());
                        m.insert("b".into(), String::new());
                        inputs = Some(m);
                    } else if self.add_type == "dff" {
                        let mut m = HashMap::new();
                        m.insert("d".into(), String::new());
                        m.insert("clk".into(), String::new());
                        inputs = Some(m);
                    } else if self.add_type != "clk" {
                        // Abstraction instance – populate inputs from definition
                        if let Some(abs) = self
                            .circuit
                            .abstractions
                            .iter()
                            .find(|a| a.name == self.add_type)
                        {
                            let mut m = HashMap::new();
                            for inp in &abs.inputs {
                                m.insert(inp.clone(), String::new());
                            }
                            inputs = Some(m);
                        }
                    }

                    let comp = ComponentDef {
                        component_type: self.add_type.clone(),
                        name: self.add_name.clone(),
                        init: if self.add_type == "dff" {
                            Some(false)
                        } else {
                            None
                        },
                        inputs,
                        pos: Some([80.0, 80.0]),
                    };
                    self.circuit.components.push(comp);
                    self.selected = Some(self.circuit.components.len() - 1);
                    self.add_name.clear();
                    self.status = "Component added".into();
                }

                // ── Selected component details ───────────────────────────
                ui.separator();
                if let Some(sel) = self.selected {
                    if sel < self.circuit.components.len() {
                        ui.heading("Selected Component");
                        let comp = &mut self.circuit.components[sel];
                        ui.horizontal(|ui| {
                            ui.label("Type:");
                            ui.label(&comp.component_type);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut comp.name);
                        });
                        if comp.component_type == "dff" {
                            let mut init_val = comp.init.unwrap_or(false);
                            if ui.checkbox(&mut init_val, "Init value").changed() {
                                comp.init = Some(init_val);
                            }
                        }
                        // Edit inputs
                        if let Some(ref mut inputs) = comp.inputs {
                            ui.label("Inputs:");
                            let keys: Vec<String> = inputs.keys().cloned().collect();
                            for key in &keys {
                                ui.horizontal(|ui| {
                                    ui.label(format!("  {key}:"));
                                    let val = inputs.get_mut(key).unwrap();
                                    ui.text_edit_singleline(val);
                                });
                            }
                        }
                        if let Some([x, y]) = &comp.pos {
                            ui.label(format!("Pos: ({:.0}, {:.0})", x, y));
                        }
                        ui.add_space(8.0);
                        if ui.button("Delete component").clicked() {
                            self.circuit.components.remove(sel);
                            self.selected = None;
                            self.status = "Component deleted".into();
                        }
                    } else {
                        self.selected = None;
                    }
                }

                // ── Simulation output ────────────────────────────────────
                if self.sim_error.is_some() || self.sim_output.is_some() {
                    ui.separator();
                    ui.heading("Simulation");
                    if let Some(ref err) = self.sim_error {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                    if let Some(ref out) = self.sim_output {
                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                ui.monospace(out);
                            });
                    }
                }
            });

        // ── Central canvas ───────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            let (response, painter) = ui.allocate_painter(
                ui.available_size(),
                egui::Sense::click_and_drag(),
            );
            let canvas_rect = response.rect;

            // Grid background
            let grid_color = egui::Color32::from_gray(40);
            let min = canvas_rect.min;
            let max = canvas_rect.max;
            let ox = self.canvas_offset.x % GRID;
            let oy = self.canvas_offset.y % GRID;
            {
                let mut x = min.x + ox;
                while x < max.x {
                    painter.line_segment(
                        [egui::pos2(x, min.y), egui::pos2(x, max.y)],
                        egui::Stroke::new(0.5, grid_color),
                    );
                    x += GRID;
                }
                let mut y = min.y + oy;
                while y < max.y {
                    painter.line_segment(
                        [egui::pos2(min.x, y), egui::pos2(max.x, y)],
                        egui::Stroke::new(0.5, grid_color),
                    );
                    y += GRID;
                }
            }

            let origin = canvas_rect.min + self.canvas_offset;

            // Handle canvas panning with middle mouse button or right-click drag
            if let Some(pointer_pos) = response.interact_pointer_pos() {
                if response.drag_started_by(egui::PointerButton::Middle)
                    || response.drag_started_by(egui::PointerButton::Secondary)
                {
                    self.canvas_drag = true;
                    self.last_canvas_pointer = Some(pointer_pos);
                }
            }
            if self.canvas_drag {
                if let Some(pointer_pos) = ctx.pointer_interact_pos() {
                    if let Some(last) = self.last_canvas_pointer {
                        let delta = pointer_pos - last;
                        self.canvas_offset += delta;
                    }
                    self.last_canvas_pointer = Some(pointer_pos);
                }
                if ctx.input(|i| {
                    i.pointer.button_released(egui::PointerButton::Middle)
                        || i.pointer.button_released(egui::PointerButton::Secondary)
                }) {
                    self.canvas_drag = false;
                    self.last_canvas_pointer = None;
                }
            }

            // Collect rects first to avoid borrow issues
            let comp_count = self.circuit.components.len();
            let rects: Vec<egui::Rect> = (0..comp_count)
                .map(|i| {
                    let pos = self.comp_pos(i);
                    let screen_pos = origin + pos.to_vec2();
                    egui::Rect::from_min_size(screen_pos, egui::vec2(COMP_W, COMP_H))
                })
                .collect();

            // Draw wires first (behind components)
            for i in 0..comp_count {
                let comp = &self.circuit.components[i];
                if let Some(ref inputs) = comp.inputs {
                    let dest_rect = rects[i];
                    let input_keys: Vec<String> = {
                        let mut keys: Vec<String> = inputs.keys().cloned().collect();
                        keys.sort();
                        keys
                    };
                    for (port_idx, key) in input_keys.iter().enumerate() {
                        let source_name = &inputs[key];
                        if source_name.is_empty() {
                            continue;
                        }
                        if let Some(src_i) = self.find_comp(source_name) {
                            let src_rect = rects[src_i];
                            // Source: right edge center
                            let src_pt = egui::pos2(
                                src_rect.max.x,
                                src_rect.min.y + src_rect.height() / 2.0,
                            );
                            // Dest: left edge, distributed by port
                            let n_ports = input_keys.len() as f32;
                            let port_y = dest_rect.min.y
                                + dest_rect.height() * (port_idx as f32 + 1.0) / (n_ports + 1.0);
                            let dest_pt = egui::pos2(dest_rect.min.x, port_y);

                            // Manhattan routing
                            let mid_x = (src_pt.x + dest_pt.x) / 2.0;
                            let points = vec![
                                src_pt,
                                egui::pos2(mid_x, src_pt.y),
                                egui::pos2(mid_x, dest_pt.y),
                                dest_pt,
                            ];
                            let wire_color = if self.selected == Some(i) || self.selected == Some(src_i) {
                                egui::Color32::from_rgb(255, 255, 100)
                            } else {
                                egui::Color32::from_gray(160)
                            };
                            for w in points.windows(2) {
                                painter.line_segment(
                                    [w[0], w[1]],
                                    egui::Stroke::new(1.5, wire_color),
                                );
                            }
                            // Port circles
                            painter.circle_filled(src_pt, PORT_R, wire_color);
                            painter.circle_filled(dest_pt, PORT_R, wire_color);
                        }
                    }
                }
            }

            // Draw components
            for i in 0..comp_count {
                let rect = rects[i];
                let comp = &self.circuit.components[i];
                let color = type_color(&comp.component_type);
                let is_selected = self.selected == Some(i);

                // Box
                let stroke = if is_selected {
                    egui::Stroke::new(2.5, egui::Color32::WHITE)
                } else {
                    egui::Stroke::new(1.0, color)
                };
                painter.rect_filled(rect, 6.0, color.linear_multiply(0.3));
                painter.rect_stroke(rect, 6.0, stroke, egui::StrokeKind::Outside);

                // Label
                let label = format!("{}\n({})", comp.name, comp.component_type);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &label,
                    egui::FontId::proportional(13.0),
                    egui::Color32::WHITE,
                );

                // Init badge for DFFs
                if comp.component_type == "dff" {
                    if let Some(init) = comp.init {
                        let badge = if init { "1" } else { "0" };
                        painter.text(
                            rect.right_top() + egui::vec2(-8.0, 8.0),
                            egui::Align2::CENTER_CENTER,
                            badge,
                            egui::FontId::monospace(11.0),
                            egui::Color32::YELLOW,
                        );
                    }
                }
            }

            // Handle clicks and drags on components
            if response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(pointer) = response.interact_pointer_pos() {
                    // Check components in reverse order (top-most first)
                    let mut found = None;
                    for i in (0..comp_count).rev() {
                        if rects[i].contains(pointer) {
                            found = Some(i);
                            break;
                        }
                    }
                    if let Some(i) = found {
                        self.selected = Some(i);
                        self.dragging = Some(i);
                        self.drag_offset = pointer - rects[i].min;
                    } else {
                        self.selected = None;
                    }
                }
            }

            if let Some(drag_i) = self.dragging {
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(pointer) = ctx.pointer_interact_pos() {
                        let new_pos = pointer - self.drag_offset - origin.to_vec2();
                        let sx = snap(new_pos.x);
                        let sy = snap(new_pos.y);
                        self.circuit.components[drag_i].pos = Some([sx, sy]);
                    }
                }
                if response.drag_stopped() {
                    self.dragging = None;
                }
            }

            // Handle click without drag (selection)
            if response.clicked() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let mut found = None;
                    for i in (0..comp_count).rev() {
                        if rects[i].contains(pointer) {
                            found = Some(i);
                            break;
                        }
                    }
                    self.selected = found;
                }
            }
        });
    }
}

// ── File dialogs (simple stdin-based fallback) ───────────────────────────────

fn rfd_open() -> Option<PathBuf> {
    // Use a simple file dialog via native dialog or fallback
    // For now, use the environment or a hardcoded examples path
    // We'll try the rfd crate if available, otherwise use a simple approach
    use std::process::Command;
    let output = Command::new("zenity")
        .args(["--file-selection", "--title=Open Circuit YAML", "--file-filter=YAML files | *.yaml *.yml"])
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    } else {
        None
    }
}

fn rfd_save() -> Option<PathBuf> {
    use std::process::Command;
    let output = Command::new("zenity")
        .args(["--file-selection", "--save", "--title=Save Circuit YAML", "--file-filter=YAML files | *.yaml *.yml"])
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    } else {
        None
    }
}
