use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;

use diycpu::compiler::compile;
use diycpu::expander::expand;
use diycpu::simulator::simulate;
use diycpu::types::{AbstractionDef, CircuitDef, ComponentDef};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]),
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
const WIRE_HI: egui::Color32 = egui::Color32::from_rgb(80, 255, 80);
const WIRE_LO: egui::Color32 = egui::Color32::from_rgb(80, 80, 80);
const WIRE_UNKNOWN: egui::Color32 = egui::Color32::from_gray(140);
const PORT_HOVER: egui::Color32 = egui::Color32::from_rgb(255, 255, 100);

fn type_color(t: &str) -> egui::Color32 {
    match t {
        "clk" => CLK_COLOR,
        "nand" => NAND_COLOR,
        "dff" => DFF_COLOR,
        _ => ABS_COLOR,
    }
}

// ── Constants ────────────────────────────────────────────────────────────────

const COMP_W: f32 = 140.0;
const COMP_H: f32 = 70.0;
const GRID: f32 = 20.0;
const PORT_R: f32 = 5.0;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 3.0;

fn snap(v: f32) -> f32 {
    (v / GRID).round() * GRID
}

// ── Port geometry ────────────────────────────────────────────────────────────

/// Returns (port_position, port_label) for each input port of a component.
fn input_ports(rect: egui::Rect, comp: &ComponentDef) -> Vec<(egui::Pos2, String)> {
    let keys = input_keys(comp);
    let n = keys.len() as f32;
    keys.iter()
        .enumerate()
        .map(|(i, k)| {
            let y = rect.min.y + rect.height() * (i as f32 + 1.0) / (n + 1.0);
            (egui::pos2(rect.min.x, y), k.clone())
        })
        .collect()
}

/// Returns the output port position (right edge center).
fn output_port(rect: egui::Rect) -> egui::Pos2 {
    egui::pos2(rect.max.x, rect.min.y + rect.height() / 2.0)
}

fn input_keys(comp: &ComponentDef) -> Vec<String> {
    if let Some(ref inputs) = comp.inputs {
        let mut keys: Vec<String> = inputs.keys().cloned().collect();
        keys.sort();
        keys
    } else {
        vec![]
    }
}

// ── Wire connection mode ─────────────────────────────────────────────────────

#[derive(Default)]
struct WireState {
    /// Source component index we're wiring from.
    from_comp: Option<usize>,
}

// ── Undo/Redo ────────────────────────────────────────────────────────────────

struct UndoStack {
    states: Vec<CircuitDef>,
    pos: usize,
    /// Tracks whether a save point has been pushed for the current action.
    dirty: bool,
}

impl UndoStack {
    fn new(initial: &CircuitDef) -> Self {
        Self {
            states: vec![initial.clone()],
            pos: 0,
            dirty: false,
        }
    }

    fn push(&mut self, state: &CircuitDef) {
        // Truncate any redo history.
        self.states.truncate(self.pos + 1);
        self.states.push(state.clone());
        self.pos = self.states.len() - 1;
        // Keep stack bounded.
        if self.states.len() > 200 {
            self.states.remove(0);
            self.pos -= 1;
        }
        self.dirty = false;
    }

    fn undo(&mut self) -> Option<&CircuitDef> {
        if self.pos > 0 {
            self.pos -= 1;
            Some(&self.states[self.pos])
        } else {
            None
        }
    }

    fn redo(&mut self) -> Option<&CircuitDef> {
        if self.pos + 1 < self.states.len() {
            self.pos += 1;
            Some(&self.states[self.pos])
        } else {
            None
        }
    }

    fn can_undo(&self) -> bool {
        self.pos > 0
    }

    fn can_redo(&self) -> bool {
        self.pos + 1 < self.states.len()
    }
}

// ── Which right-panel tab is active ──────────────────────────────────────────

#[derive(PartialEq, Clone, Copy)]
enum PanelTab {
    Components,
    Abstractions,
}

// ── App state ────────────────────────────────────────────────────────────────

struct App {
    circuit: CircuitDef,
    file_path: Option<PathBuf>,
    selected: Option<usize>,
    dragging: Option<usize>,
    drag_offset: egui::Vec2,
    canvas_offset: egui::Vec2,
    zoom: f32,
    canvas_drag: bool,
    last_canvas_pointer: Option<egui::Pos2>,

    // Add-component dialog
    add_type: String,
    add_name: String,

    // Wire drawing
    wire: WireState,

    // Undo/redo
    undo: UndoStack,

    // Right panel tab
    panel_tab: PanelTab,

    // Abstraction editor
    abs_edit_name: String,
    abs_edit_inputs: String,
    abs_edit_outputs_key: String,
    abs_edit_outputs_val: String,
    abs_selected: Option<usize>,

    // Simulation
    sim_results: Option<Vec<HashMap<String, bool>>>,
    sim_step: usize,
    sim_error: Option<String>,

    // Status bar message
    status: String,
}

impl Default for App {
    fn default() -> Self {
        let circuit = CircuitDef {
            name: "untitled".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![],
        };
        Self {
            undo: UndoStack::new(&circuit),
            circuit,
            file_path: None,
            selected: None,
            dragging: None,
            drag_offset: egui::Vec2::ZERO,
            canvas_offset: egui::Vec2::ZERO,
            zoom: 1.0,
            canvas_drag: false,
            last_canvas_pointer: None,
            add_type: "nand".into(),
            add_name: String::new(),
            wire: WireState::default(),
            panel_tab: PanelTab::Components,
            abs_edit_name: String::new(),
            abs_edit_inputs: String::new(),
            abs_edit_outputs_key: String::new(),
            abs_edit_outputs_val: String::new(),
            abs_selected: None,
            sim_results: None,
            sim_step: 0,
            sim_error: None,
            status: "Ready — scroll to zoom, right-drag to pan".into(),
        }
    }
}

impl App {
    fn checkpoint(&mut self) {
        self.undo.push(&self.circuit);
    }

    fn undo(&mut self) {
        if let Some(state) = self.undo.undo() {
            self.circuit = state.clone();
            self.selected = None;
            self.status = "Undo".into();
        }
    }

    fn redo(&mut self) {
        if let Some(state) = self.undo.redo() {
            self.circuit = state.clone();
            self.selected = None;
            self.status = "Redo".into();
        }
    }

    /// Position for component i (uses stored pos or auto-layout).
    fn comp_pos(&self, i: usize) -> egui::Pos2 {
        let c = &self.circuit.components[i];
        if let Some([x, y]) = c.pos {
            egui::pos2(x, y)
        } else {
            let cols = 4;
            let col = i % cols;
            let row = i / cols;
            egui::pos2(
                80.0 + col as f32 * (COMP_W + 50.0),
                80.0 + row as f32 * (COMP_H + 70.0),
            )
        }
    }

    fn find_comp(&self, name: &str) -> Option<usize> {
        self.circuit
            .components
            .iter()
            .position(|c| c.name == name)
    }

    fn run_sim(&mut self) {
        self.sim_results = None;
        self.sim_error = None;
        self.sim_step = 0;

        let expanded = match expand(&self.circuit) {
            Ok(e) => e,
            Err(e) => {
                self.sim_error = Some(format!("Expand: {e}"));
                return;
            }
        };

        let compiled = match compile(&expanded.def) {
            Ok(c) => c,
            Err(e) => {
                self.sim_error = Some(format!("Compile: {e}"));
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

        self.sim_results = Some(results);
        self.status = format!("Simulated {} steps", self.circuit.steps);
    }

    /// Get the value of a component at the current sim step.
    fn sim_value(&self, name: &str) -> Option<bool> {
        self.sim_results
            .as_ref()
            .and_then(|r| r.get(self.sim_step))
            .and_then(|step| step.get(name).copied())
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
            self.status = "No file path — use Save As".into();
        }
    }

    fn load_yaml(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_yaml::from_str::<CircuitDef>(&contents) {
                Ok(def) => {
                    self.circuit = def;
                    self.file_path = Some(path.clone());
                    self.selected = None;
                    self.sim_results = None;
                    self.sim_error = None;
                    self.sim_step = 0;
                    self.undo = UndoStack::new(&self.circuit);
                    self.status = format!("Loaded {}", path.display());
                }
                Err(e) => self.status = format!("YAML parse error: {e}"),
            },
            Err(e) => self.status = format!("Read error: {e}"),
        }
    }

    fn new_circuit(&mut self) {
        self.circuit = CircuitDef {
            name: "untitled".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![],
        };
        self.file_path = None;
        self.selected = None;
        self.sim_results = None;
        self.sim_error = None;
        self.sim_step = 0;
        self.undo = UndoStack::new(&self.circuit);
        self.status = "New circuit".into();
    }

    fn delete_selected(&mut self) {
        if let Some(sel) = self.selected.take() {
            if sel < self.circuit.components.len() {
                self.checkpoint();
                self.circuit.components.remove(sel);
                self.status = "Component deleted".into();
            }
        }
    }

    fn duplicate_selected(&mut self) {
        if let Some(sel) = self.selected {
            if sel < self.circuit.components.len() {
                self.checkpoint();
                let mut dup = self.circuit.components[sel].clone();
                dup.name = format!("{}_copy", dup.name);
                // Offset position
                if let Some([x, y]) = dup.pos {
                    dup.pos = Some([x + COMP_W + 20.0, y]);
                }
                self.circuit.components.push(dup);
                self.selected = Some(self.circuit.components.len() - 1);
                self.status = "Component duplicated".into();
            }
        }
    }

    /// Screen rect for component i at the given origin and zoom.
    fn screen_rect(&self, i: usize, origin: egui::Pos2) -> egui::Rect {
        let pos = self.comp_pos(i);
        let screen_pos = origin + pos.to_vec2() * self.zoom;
        egui::Rect::from_min_size(
            screen_pos,
            egui::vec2(COMP_W * self.zoom, COMP_H * self.zoom),
        )
    }
}

// ── egui rendering ───────────────────────────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── Keyboard shortcuts ───────────────────────────────────────────
        let modifiers = ctx.input(|i| i.modifiers);
        let ctrl = modifiers.ctrl || modifiers.mac_cmd;

        if ctx.input(|i| i.key_pressed(egui::Key::S)) && ctrl {
            self.save_yaml();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::O)) && ctrl {
            if let Some(path) = rfd_open() {
                self.load_yaml(path);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::N)) && ctrl {
            self.new_circuit();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Z)) && ctrl && !modifiers.shift {
            self.undo();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Y)) && ctrl {
            self.redo();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Z)) && ctrl && modifiers.shift {
            self.redo();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::D)) && ctrl {
            self.duplicate_selected();
        }
        // Delete key — only if no text field is focused.
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && ctx.memory(|m| m.focused().is_none())
        {
            self.delete_selected();
        }

        // ── Top menu bar ─────────────────────────────────────────────────
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New          Ctrl+N").clicked() {
                        self.new_circuit();
                        ui.close();
                    }
                    if ui.button("Open...      Ctrl+O").clicked() {
                        if let Some(path) = rfd_open() {
                            self.load_yaml(path);
                        }
                        ui.close();
                    }
                    if ui.button("Save         Ctrl+S").clicked() {
                        self.save_yaml();
                        ui.close();
                    }
                    if ui.button("Save As...").clicked() {
                        if let Some(path) = rfd_save() {
                            self.file_path = Some(path);
                            self.save_yaml();
                        }
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let undo_label = if self.undo.can_undo() {
                        "Undo         Ctrl+Z"
                    } else {
                        "Undo         (nothing)"
                    };
                    if ui.add_enabled(self.undo.can_undo(), egui::Button::new(undo_label)).clicked() {
                        self.undo();
                        ui.close();
                    }
                    let redo_label = if self.undo.can_redo() {
                        "Redo         Ctrl+Y"
                    } else {
                        "Redo         (nothing)"
                    };
                    if ui.add_enabled(self.undo.can_redo(), egui::Button::new(redo_label)).clicked() {
                        self.redo();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new("Duplicate    Ctrl+D"))
                        .clicked()
                    {
                        self.duplicate_selected();
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.selected.is_some(), egui::Button::new("Delete       Del"))
                        .clicked()
                    {
                        self.delete_selected();
                        ui.close();
                    }
                });
                ui.separator();
                if ui.button("Simulate").clicked() {
                    self.run_sim();
                }
                if self.wire.from_comp.is_some() {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Wiring... click an input port (Esc to cancel)",
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(&self.status);
                });
            });
        });

        // ── Bottom panel: simulation playback ────────────────────────────
        if self.sim_results.is_some() {
            egui::TopBottomPanel::bottom("sim_playback")
                .min_height(36.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let max_step = self
                            .sim_results
                            .as_ref()
                            .map(|r| r.len().saturating_sub(1))
                            .unwrap_or(0);
                        ui.label("Step:");
                        ui.add(
                            egui::Slider::new(&mut self.sim_step, 0..=max_step)
                                .text(format!("/ {max_step}")),
                        );
                        if ui.button("|<").clicked() {
                            self.sim_step = 0;
                        }
                        if ui.button("<").clicked() {
                            self.sim_step = self.sim_step.saturating_sub(1);
                        }
                        if ui.button(">").clicked() {
                            if self.sim_step < max_step {
                                self.sim_step += 1;
                            }
                        }
                        if ui.button(">|").clicked() {
                            self.sim_step = max_step;
                        }
                        if ui.button("Clear sim").clicked() {
                            self.sim_results = None;
                            self.sim_step = 0;
                        }
                    });
                });
        }

        // ── Escape cancels wire mode ─────────────────────────────────────
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.wire.from_comp.is_some() {
                self.wire.from_comp = None;
                self.status = "Wire cancelled".into();
            }
        }

        // ── Right panel ──────────────────────────────────────────────────
        egui::SidePanel::right("props")
            .min_width(300.0)
            .show(ctx, |ui| {
                // Tab bar
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.panel_tab, PanelTab::Components, "Components");
                    ui.selectable_value(
                        &mut self.panel_tab,
                        PanelTab::Abstractions,
                        "Abstractions",
                    );
                });
                ui.separator();

                match self.panel_tab {
                    PanelTab::Components => self.draw_components_panel(ui),
                    PanelTab::Abstractions => self.draw_abstractions_panel(ui),
                }

                // ── Sim error / results ──────────────────────────────────
                if self.sim_error.is_some() || self.sim_results.is_some() {
                    ui.separator();
                    ui.heading("Simulation");
                    if let Some(ref err) = self.sim_error {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                    if let Some(ref results) = self.sim_results {
                        // Show current step values
                        if let Some(step) = results.get(self.sim_step) {
                            let mut entries: Vec<_> = step.iter().collect();
                            entries.sort_by_key(|(k, _)| (*k).clone());
                            egui::ScrollArea::vertical()
                                .max_height(400.0)
                                .show(ui, |ui| {
                                    egui::Grid::new("sim_values")
                                        .striped(true)
                                        .show(ui, |ui| {
                                            ui.strong("Signal");
                                            ui.strong("Value");
                                            ui.end_row();
                                            for (name, &val) in &entries {
                                                // Skip internal expanded names
                                                if name.contains('/') {
                                                    continue;
                                                }
                                                ui.label(*name);
                                                let (text, color) = if val {
                                                    ("1", WIRE_HI)
                                                } else {
                                                    ("0", WIRE_LO)
                                                };
                                                ui.colored_label(color, text);
                                                ui.end_row();
                                            }
                                        });
                                });
                        }
                    }
                }
            });

        // ── Central canvas ───────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_canvas(ui, ctx);
        });
    }
}

impl App {
    // ── Components panel ─────────────────────────────────────────────────

    fn draw_components_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Circuit");
        ui.horizontal(|ui| {
            ui.label("Name:");
            if ui.text_edit_singleline(&mut self.circuit.name).lost_focus() {
                self.checkpoint();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Steps:");
            if ui
                .add(egui::DragValue::new(&mut self.circuit.steps).range(1..=10000))
                .changed()
            {
                self.checkpoint();
            }
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
            self.checkpoint();
            let inputs = make_default_inputs(&self.add_type, &self.circuit.abstractions);
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

        // ── Selected component details ───────────────────────────────
        ui.separator();
        if let Some(sel) = self.selected {
            if sel < self.circuit.components.len() {
                ui.heading("Selected Component");
                let comp = &mut self.circuit.components[sel];
                ui.horizontal(|ui| {
                    ui.label("Type:");
                    ui.strong(&comp.component_type);
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
                    ui.label("Inputs (type component name, or click ports):");
                    let keys: Vec<String> = {
                        let mut k: Vec<_> = inputs.keys().cloned().collect();
                        k.sort();
                        k
                    };
                    for key in &keys {
                        ui.horizontal(|ui| {
                            ui.label(format!("  {key}:"));
                            let val = inputs.get_mut(key).unwrap();
                            ui.text_edit_singleline(val);
                        });
                    }
                }

                // Show sim value if available
                let name = comp.name.clone();
                if let Some(val) = self.sim_value(&name) {
                    let (text, color) = if val { ("= 1", WIRE_HI) } else { ("= 0", WIRE_LO) };
                    ui.colored_label(color, text);
                }

                if let Some([x, y]) = &self.circuit.components[sel].pos {
                    ui.label(format!("Pos: ({:.0}, {:.0})", x, y));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        self.delete_selected();
                    }
                    if ui.button("Duplicate").clicked() {
                        self.duplicate_selected();
                    }
                });
            } else {
                self.selected = None;
            }
        } else {
            ui.label("Click a component to select it.");
            ui.label("Click an output port to start wiring.");
        }
    }

    // ── Abstractions panel ───────────────────────────────────────────────

    fn draw_abstractions_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Abstractions");

        // List existing abstractions
        let abs_names: Vec<String> = self
            .circuit
            .abstractions
            .iter()
            .map(|a| a.name.clone())
            .collect();

        if abs_names.is_empty() {
            ui.label("No abstractions defined.");
        } else {
            for (i, name) in abs_names.iter().enumerate() {
                let selected = self.abs_selected == Some(i);
                if ui.selectable_label(selected, name).clicked() {
                    self.abs_selected = Some(i);
                }
            }
        }

        // Show selected abstraction details
        if let Some(sel) = self.abs_selected {
            if sel < self.circuit.abstractions.len() {
                ui.separator();
                let abs = &self.circuit.abstractions[sel];
                ui.strong(&abs.name);
                ui.label(format!("Inputs: {}", abs.inputs.join(", ")));
                ui.label("Outputs:");
                for (k, v) in &abs.outputs {
                    ui.label(format!("  {k} -> {v}"));
                }
                ui.label(format!("Components: {}", abs.components.len()));
                for comp in &abs.components {
                    ui.label(format!("  {} ({})", comp.name, comp.component_type));
                }
                ui.add_space(4.0);
                if ui.button("Delete abstraction").clicked() {
                    self.checkpoint();
                    self.circuit.abstractions.remove(sel);
                    self.abs_selected = None;
                    self.status = "Abstraction deleted".into();
                }
            }
        }

        // Create new abstraction
        ui.separator();
        ui.heading("New Abstraction");
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.abs_edit_name);
        });
        ui.horizontal(|ui| {
            ui.label("Inputs (comma-separated):");
            ui.text_edit_singleline(&mut self.abs_edit_inputs);
        });
        ui.label("Output mapping:");
        ui.horizontal(|ui| {
            ui.label("  key:");
            ui.text_edit_singleline(&mut self.abs_edit_outputs_key);
            ui.label("val:");
            ui.text_edit_singleline(&mut self.abs_edit_outputs_val);
        });
        if ui.button("Create abstraction").clicked() && !self.abs_edit_name.is_empty() {
            self.checkpoint();
            let inputs: Vec<String> = self
                .abs_edit_inputs
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let mut outputs = HashMap::new();
            if !self.abs_edit_outputs_key.is_empty() {
                outputs.insert(
                    self.abs_edit_outputs_key.clone(),
                    self.abs_edit_outputs_val.clone(),
                );
            }
            self.circuit.abstractions.push(AbstractionDef {
                name: self.abs_edit_name.clone(),
                inputs,
                outputs,
                components: vec![],
            });
            self.abs_selected = Some(self.circuit.abstractions.len() - 1);
            self.abs_edit_name.clear();
            self.abs_edit_inputs.clear();
            self.abs_edit_outputs_key.clear();
            self.abs_edit_outputs_val.clear();
            self.status = "Abstraction created".into();
        }
    }

    // ── Canvas drawing ───────────────────────────────────────────────────

    fn draw_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let canvas_rect = response.rect;

        // ── Zoom ─────────────────────────────────────────────────────
        let scroll_delta = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll_delta != 0.0 && canvas_rect.contains(ctx.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
            let factor = if scroll_delta > 0.0 { 1.1 } else { 1.0 / 1.1 };
            let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
            // Zoom towards pointer position
            if let Some(pointer) = ctx.input(|i| i.pointer.hover_pos()) {
                let before = (pointer - canvas_rect.min - self.canvas_offset) / self.zoom;
                self.zoom = new_zoom;
                let after = before * self.zoom;
                self.canvas_offset = pointer - canvas_rect.min - after;
            } else {
                self.zoom = new_zoom;
            }
        }

        // ── Grid background ──────────────────────────────────────────
        let grid_color = egui::Color32::from_gray(40);
        let scaled_grid = GRID * self.zoom;
        if scaled_grid > 4.0 {
            let ox = self.canvas_offset.x.rem_euclid(scaled_grid);
            let oy = self.canvas_offset.y.rem_euclid(scaled_grid);
            let mut x = canvas_rect.min.x + ox;
            while x < canvas_rect.max.x {
                painter.line_segment(
                    [egui::pos2(x, canvas_rect.min.y), egui::pos2(x, canvas_rect.max.y)],
                    egui::Stroke::new(0.5, grid_color),
                );
                x += scaled_grid;
            }
            let mut y = canvas_rect.min.y + oy;
            while y < canvas_rect.max.y {
                painter.line_segment(
                    [egui::pos2(canvas_rect.min.x, y), egui::pos2(canvas_rect.max.x, y)],
                    egui::Stroke::new(0.5, grid_color),
                );
                y += scaled_grid;
            }
        }

        let origin = canvas_rect.min + self.canvas_offset;

        // ── Panning ──────────────────────────────────────────────────
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
                    self.canvas_offset += pointer_pos - last;
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

        // ── Compute screen rects ─────────────────────────────────────
        let comp_count = self.circuit.components.len();
        let rects: Vec<egui::Rect> = (0..comp_count)
            .map(|i| self.screen_rect(i, origin))
            .collect();

        // Current pointer position (for hover effects)
        let hover_pos = ctx.input(|i| i.pointer.hover_pos());
        let sim_active = self.sim_results.is_some();

        // ── Draw wires ───────────────────────────────────────────────
        for i in 0..comp_count {
            let comp = &self.circuit.components[i];
            if let Some(ref inputs) = comp.inputs {
                let dest_rect = rects[i];
                let keys = input_keys(comp);
                let n_ports = keys.len() as f32;

                for (port_idx, key) in keys.iter().enumerate() {
                    let source_name = &inputs[key];
                    if source_name.is_empty() {
                        continue;
                    }
                    if let Some(src_i) = self.find_comp(source_name) {
                        let src_rect = rects[src_i];
                        let src_pt = output_port(src_rect);
                        let port_y = dest_rect.min.y
                            + dest_rect.height() * (port_idx as f32 + 1.0) / (n_ports + 1.0);
                        let dest_pt = egui::pos2(dest_rect.min.x, port_y);

                        // Wire color based on sim value
                        let wire_color = if sim_active {
                            match self.sim_value(source_name) {
                                Some(true) => WIRE_HI,
                                Some(false) => WIRE_LO,
                                None => WIRE_UNKNOWN,
                            }
                        } else if self.selected == Some(i) || self.selected == Some(src_i) {
                            egui::Color32::from_rgb(255, 255, 100)
                        } else {
                            WIRE_UNKNOWN
                        };

                        // Manhattan routing
                        let mid_x = (src_pt.x + dest_pt.x) / 2.0;
                        let points = [
                            src_pt,
                            egui::pos2(mid_x, src_pt.y),
                            egui::pos2(mid_x, dest_pt.y),
                            dest_pt,
                        ];
                        let thickness = if sim_active { 2.0 * self.zoom } else { 1.5 * self.zoom };
                        for w in points.windows(2) {
                            painter.line_segment(
                                [w[0], w[1]],
                                egui::Stroke::new(thickness, wire_color),
                            );
                        }
                        painter.circle_filled(src_pt, PORT_R * self.zoom * 0.8, wire_color);
                        painter.circle_filled(dest_pt, PORT_R * self.zoom * 0.8, wire_color);
                    }
                }
            }
        }

        // ── Draw wire-in-progress ────────────────────────────────────
        if let Some(from_i) = self.wire.from_comp {
            if from_i < rects.len() {
                let src_pt = output_port(rects[from_i]);
                if let Some(pointer) = hover_pos {
                    painter.line_segment(
                        [src_pt, pointer],
                        egui::Stroke::new(2.0, PORT_HOVER),
                    );
                    painter.circle_filled(src_pt, PORT_R * self.zoom, PORT_HOVER);
                }
            }
        }

        // ── Draw components ──────────────────────────────────────────
        let font_size = (13.0 * self.zoom).max(7.0);
        let port_font = (10.0 * self.zoom).max(6.0);

        for i in 0..comp_count {
            let rect = rects[i];
            let comp = &self.circuit.components[i];
            let color = type_color(&comp.component_type);
            let is_selected = self.selected == Some(i);

            // Background fill — tint with sim value
            let fill = if sim_active {
                match self.sim_value(&comp.name) {
                    Some(true) => egui::Color32::from_rgba_premultiplied(40, 100, 40, 200),
                    Some(false) => egui::Color32::from_rgba_premultiplied(30, 30, 30, 200),
                    None => color.linear_multiply(0.3),
                }
            } else {
                color.linear_multiply(0.3)
            };
            painter.rect_filled(rect, 6.0 * self.zoom, fill);

            // Border
            let stroke = if is_selected {
                egui::Stroke::new(2.5 * self.zoom, egui::Color32::WHITE)
            } else {
                egui::Stroke::new(1.0 * self.zoom, color)
            };
            painter.rect_stroke(rect, 6.0 * self.zoom, stroke, egui::StrokeKind::Outside);

            // Label
            let label = format!("{}\n({})", comp.name, comp.component_type);
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                &label,
                egui::FontId::proportional(font_size),
                egui::Color32::WHITE,
            );

            // Sim value badge
            if sim_active {
                if let Some(val) = self.sim_value(&comp.name) {
                    let badge = if val { "1" } else { "0" };
                    let badge_color = if val { WIRE_HI } else { WIRE_LO };
                    painter.text(
                        rect.right_top() + egui::vec2(-10.0 * self.zoom, 10.0 * self.zoom),
                        egui::Align2::CENTER_CENTER,
                        badge,
                        egui::FontId::monospace((14.0 * self.zoom).max(8.0)),
                        badge_color,
                    );
                }
            } else if comp.component_type == "dff" {
                // Init badge
                if let Some(init) = comp.init {
                    let badge = if init { "1" } else { "0" };
                    painter.text(
                        rect.right_top() + egui::vec2(-8.0 * self.zoom, 8.0 * self.zoom),
                        egui::Align2::CENTER_CENTER,
                        badge,
                        egui::FontId::monospace((11.0 * self.zoom).max(6.0)),
                        egui::Color32::YELLOW,
                    );
                }
            }

            // ── Port labels ──────────────────────────────────────────
            // Input ports (left side)
            let ports = input_ports(rect, comp);
            for (pt, label) in &ports {
                // Port circle
                let hovering = hover_pos
                    .map(|hp| hp.distance(*pt) < PORT_R * self.zoom * 2.5)
                    .unwrap_or(false);
                let pcol = if hovering { PORT_HOVER } else { color };
                painter.circle_filled(*pt, PORT_R * self.zoom, pcol);
                // Label
                painter.text(
                    *pt + egui::vec2(8.0 * self.zoom, 0.0),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(port_font),
                    egui::Color32::from_gray(200),
                );
            }

            // Output port (right side)
            let out_pt = output_port(rect);
            let out_hovering = hover_pos
                .map(|hp| hp.distance(out_pt) < PORT_R * self.zoom * 2.5)
                .unwrap_or(false);
            let out_col = if out_hovering { PORT_HOVER } else { color };
            painter.circle_filled(out_pt, PORT_R * self.zoom, out_col);
            painter.text(
                out_pt + egui::vec2(-8.0 * self.zoom, 0.0),
                egui::Align2::RIGHT_CENTER,
                "out",
                egui::FontId::proportional(port_font),
                egui::Color32::from_gray(200),
            );
        }

        // ── Interaction: clicks & drags ──────────────────────────────

        // Detect port clicks for wiring
        if response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let port_radius = PORT_R * self.zoom * 2.5;

                if self.wire.from_comp.is_some() {
                    // We're in wire mode — look for an input port to connect to
                    let mut connected = false;
                    'outer: for i in 0..comp_count {
                        let ports = input_ports(rects[i], &self.circuit.components[i]);
                        for (pt, key) in &ports {
                            if pointer.distance(*pt) < port_radius {
                                // Connect!
                                let from_i = self.wire.from_comp.unwrap();
                                let source_name =
                                    self.circuit.components[from_i].name.clone();
                                self.checkpoint();
                                let comp = &mut self.circuit.components[i];
                                if comp.inputs.is_none() {
                                    comp.inputs = Some(HashMap::new());
                                }
                                comp.inputs.as_mut().unwrap().insert(key.clone(), source_name.clone());
                                self.wire.from_comp = None;
                                self.status = format!("Connected {source_name} -> {}.{key}", comp.name);
                                connected = true;
                                break 'outer;
                            }
                        }
                    }
                    if !connected {
                        // Click on empty space cancels wire
                        self.wire.from_comp = None;
                        self.status = "Wire cancelled".into();
                    }
                } else {
                    // Check if clicking an output port to start wiring
                    let mut started_wire = false;
                    for i in (0..comp_count).rev() {
                        let out_pt = output_port(rects[i]);
                        if pointer.distance(out_pt) < port_radius {
                            self.wire.from_comp = Some(i);
                            self.status = format!(
                                "Wiring from '{}' — click an input port",
                                self.circuit.components[i].name
                            );
                            started_wire = true;
                            break;
                        }
                    }
                    if !started_wire {
                        // Normal click selection
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
            }
        }

        // Component dragging
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pointer) = response.interact_pointer_pos() {
                // Only start drag if not on a port
                let port_radius = PORT_R * self.zoom * 2.5;
                let on_port = (0..comp_count).any(|i| {
                    let out_pt = output_port(rects[i]);
                    if pointer.distance(out_pt) < port_radius {
                        return true;
                    }
                    let ports = input_ports(rects[i], &self.circuit.components[i]);
                    ports.iter().any(|(pt, _)| pointer.distance(*pt) < port_radius)
                });

                if !on_port {
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
                    }
                }
            }
        }

        if let Some(drag_i) = self.dragging {
            if response.dragged_by(egui::PointerButton::Primary) {
                if let Some(pointer) = ctx.pointer_interact_pos() {
                    let new_pos = (pointer - self.drag_offset - origin.to_vec2()) / self.zoom;
                    let sx = snap(new_pos.x);
                    let sy = snap(new_pos.y);
                    self.circuit.components[drag_i].pos = Some([sx, sy]);
                }
            }
            if response.drag_stopped() {
                self.checkpoint();
                self.dragging = None;
            }
        }

        // ── Zoom indicator ───────────────────────────────────────────
        painter.text(
            canvas_rect.right_bottom() + egui::vec2(-10.0, -10.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("{:.0}%", self.zoom * 100.0),
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(100),
        );
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn make_default_inputs(
    type_name: &str,
    abstractions: &[AbstractionDef],
) -> Option<HashMap<String, String>> {
    match type_name {
        "clk" => None,
        "nand" => {
            let mut m = HashMap::new();
            m.insert("a".into(), String::new());
            m.insert("b".into(), String::new());
            Some(m)
        }
        "dff" => {
            let mut m = HashMap::new();
            m.insert("d".into(), String::new());
            m.insert("clk".into(), String::new());
            Some(m)
        }
        _ => {
            if let Some(abs) = abstractions.iter().find(|a| a.name == type_name) {
                let mut m = HashMap::new();
                for inp in &abs.inputs {
                    m.insert(inp.clone(), String::new());
                }
                Some(m)
            } else {
                None
            }
        }
    }
}

// ── File dialogs ─────────────────────────────────────────────────────────────

fn rfd_open() -> Option<PathBuf> {
    use std::process::Command;
    let output = Command::new("zenity")
        .args([
            "--file-selection",
            "--title=Open Circuit YAML",
            "--file-filter=YAML files | *.yaml *.yml",
        ])
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
        .args([
            "--file-selection",
            "--save",
            "--title=Save Circuit YAML",
            "--file-filter=YAML files | *.yaml *.yml",
        ])
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
