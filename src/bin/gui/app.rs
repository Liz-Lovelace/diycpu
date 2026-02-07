use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;

use diycpu::compiler::compile;
use diycpu::expander::expand;
use diycpu::simulator::simulate;
use diycpu::types::CircuitDef;

use diycpu::types::ComponentDef;

use super::style::{COMPONENT_HEIGHT, COMPONENT_WIDTH, PORT_SPACING};
use super::undo::UndoStack;

// ── Wire connection mode ────────────────────────────────────────────────────

/// Tracks the state of an in-progress wire connection.
/// When the user clicks an output port, we store the source component index.
/// The next click on an input port completes the connection.
#[derive(Default)]
pub struct WireState {
    pub source_component_index: Option<usize>,
    /// Which output port the wire originates from (e.g. "out" or a named output).
    pub source_output_name: Option<String>,
}

// ── Panel tabs ──────────────────────────────────────────────────────────────

/// Which tab is currently active in the right side panel.
#[derive(PartialEq, Clone, Copy)]
pub enum PanelTab {
    Components,
    Abstractions,
}

// ── Main application state ──────────────────────────────────────────────────

/// The main application state for the circuit editor GUI.
pub struct App {
    /// The circuit definition being edited.
    pub circuit: CircuitDef,
    /// Path to the currently open file (if any).
    pub file_path: Option<PathBuf>,

    /// Index of the currently selected component (if any).
    pub selected_component: Option<usize>,
    /// Index of the component currently being dragged (if any).
    pub dragging_component: Option<usize>,
    /// Offset from the pointer to the top-left corner of the dragged component.
    pub drag_offset: egui::Vec2,

    /// Canvas pan offset in screen pixels.
    pub canvas_offset: egui::Vec2,
    /// Current zoom level (1.0 = 100%).
    pub zoom: f32,
    /// Whether the canvas is currently being panned (middle/right mouse drag).
    pub is_panning: bool,
    /// Last known pointer position during a pan operation.
    pub last_pan_pointer_position: Option<egui::Pos2>,

    /// Type selected in the "add component" dropdown.
    pub new_component_type: String,
    /// Name entered in the "add component" text field.
    pub new_component_name: String,

    /// State for interactive wire drawing.
    pub wire_state: WireState,

    /// Undo/redo history stack.
    pub undo_stack: UndoStack,

    /// Which tab is active in the right panel.
    pub active_panel_tab: PanelTab,

    /// Fields for the "new abstraction" editor form.
    pub abstraction_editor_name: String,
    pub abstraction_editor_inputs: String,
    /// Multiple output pairs for new abstraction form: [(key, value), ...]
    pub abstraction_editor_outputs: Vec<(String, String)>,
    /// Index of the selected abstraction in the abstractions panel.
    pub selected_abstraction_index: Option<usize>,

    /// If Some, we are editing inside an abstraction (sub-circuit mode).
    pub editing_abstraction_index: Option<usize>,
    /// Currently dragged waypoint: (component_index, input_key, waypoint_index).
    pub dragging_waypoint: Option<(usize, String, usize)>,

    /// Simulation results (one HashMap per step), or None if no simulation has run.
    pub simulation_results: Option<Vec<HashMap<String, bool>>>,
    /// The currently displayed simulation step index.
    pub simulation_step: usize,
    /// Error message from the last simulation attempt, if any.
    pub simulation_error: Option<String>,

    /// Status bar message shown at the top right of the menu bar.
    pub status_message: String,
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
            undo_stack: UndoStack::new(&circuit),
            circuit,
            file_path: None,
            selected_component: None,
            dragging_component: None,
            drag_offset: egui::Vec2::ZERO,
            canvas_offset: egui::Vec2::ZERO,
            zoom: 1.0,
            is_panning: false,
            last_pan_pointer_position: None,
            new_component_type: "nand".into(),
            new_component_name: String::new(),
            wire_state: WireState::default(),
            active_panel_tab: PanelTab::Components,
            abstraction_editor_name: String::new(),
            abstraction_editor_inputs: String::new(),
            abstraction_editor_outputs: vec![("out".into(), String::new())],
            selected_abstraction_index: None,
            editing_abstraction_index: None,
            dragging_waypoint: None,
            simulation_results: None,
            simulation_step: 0,
            simulation_error: None,
            status_message: "Ready — scroll to zoom, right-drag to pan".into(),
        }
    }
}

// ── Core methods ────────────────────────────────────────────────────────────

impl App {
    /// Returns a reference to the components currently being edited.
    /// When editing inside an abstraction, returns that abstraction's components.
    pub fn active_components(&self) -> &Vec<ComponentDef> {
        if let Some(abs_idx) = self.editing_abstraction_index {
            &self.circuit.abstractions[abs_idx].components
        } else {
            &self.circuit.components
        }
    }

    /// Returns a mutable reference to the components currently being edited.
    pub fn active_components_mut(&mut self) -> &mut Vec<ComponentDef> {
        if let Some(abs_idx) = self.editing_abstraction_index {
            &mut self.circuit.abstractions[abs_idx].components
        } else {
            &mut self.circuit.components
        }
    }

    /// Save the current circuit state to the undo stack.
    pub fn save_undo_checkpoint(&mut self) {
        self.undo_stack.push(&self.circuit);
    }

    /// Undo the last change, restoring the previous circuit state.
    pub fn undo_last_change(&mut self) {
        if let Some(previous_state) = self.undo_stack.undo() {
            self.circuit = previous_state.clone();
            self.selected_component = None;
            self.status_message = "Undo".into();
        }
    }

    /// Redo the last undone change.
    pub fn redo_last_change(&mut self) {
        if let Some(next_state) = self.undo_stack.redo() {
            self.circuit = next_state.clone();
            self.selected_component = None;
            self.status_message = "Redo".into();
        }
    }

    /// Get the canvas position for a component by index.
    /// Uses the stored position if available, otherwise auto-layouts in a grid.
    pub fn component_position(&self, component_index: usize) -> egui::Pos2 {
        let component = &self.active_components()[component_index];
        if let Some([x, y]) = component.pos {
            egui::pos2(x, y)
        } else {
            let columns = 4;
            let column = component_index % columns;
            let row = component_index / columns;
            egui::pos2(
                80.0 + column as f32 * (COMPONENT_WIDTH + 50.0),
                80.0 + row as f32 * (COMPONENT_HEIGHT + 70.0),
            )
        }
    }

    /// Compute world-space size for a component, scaling height to fit ports.
    pub fn component_size(&self, component_index: usize) -> egui::Vec2 {
        let component = &self.active_components()[component_index];
        let input_count = component.inputs.as_ref().map(|i| i.len()).unwrap_or(0);
        let output_count = self
            .circuit
            .abstractions
            .iter()
            .find(|a| a.name == component.component_type)
            .map(|a| a.outputs.len())
            .unwrap_or(1);
        let port_count = input_count.max(output_count).max(1);
        let min_height = COMPONENT_HEIGHT;
        let needed_height = (port_count as f32 + 1.0) * PORT_SPACING;
        egui::vec2(COMPONENT_WIDTH, min_height.max(needed_height))
    }

    /// Compute the screen-space rectangle for a component, applying zoom and pan.
    pub fn component_screen_rect(
        &self,
        component_index: usize,
        canvas_origin: egui::Pos2,
    ) -> egui::Rect {
        let position = self.component_position(component_index);
        let size = self.component_size(component_index);
        let screen_position = canvas_origin + position.to_vec2() * self.zoom;
        egui::Rect::from_min_size(screen_position, size * self.zoom)
    }

    /// Run the simulation and store the results.
    pub fn run_simulation(&mut self) {
        self.simulation_results = None;
        self.simulation_error = None;
        self.simulation_step = 0;

        let expanded = match expand(&self.circuit) {
            Ok(expanded) => expanded,
            Err(error) => {
                self.simulation_error = Some(format!("Expand: {error}"));
                return;
            }
        };

        let compiled = match compile(&expanded.def) {
            Ok(compiled) => compiled,
            Err(error) => {
                self.simulation_error = Some(format!("Compile: {error}"));
                return;
            }
        };

        let mut results = simulate(&compiled);

        // Add output aliases so abstraction outputs are accessible by dot-notation.
        for step in &mut results {
            for (alias, target) in &expanded.aliases {
                if let Some(&value) = step.get(target) {
                    step.insert(alias.clone(), value);
                }
            }
        }

        self.simulation_results = Some(results);
        self.status_message = format!("Simulated {} steps", self.circuit.steps);
    }

    /// Get the value of a component at the current simulation step.
    pub fn simulation_value(&self, component_name: &str) -> Option<bool> {
        self.simulation_results
            .as_ref()
            .and_then(|results| results.get(self.simulation_step))
            .and_then(|step_values| step_values.get(component_name).copied())
    }

    /// Save the circuit to the current file path as YAML.
    pub fn save_circuit_to_yaml(&mut self) {
        if let Some(path) = &self.file_path {
            match serde_yaml::to_string(&self.circuit) {
                Ok(yaml) => match std::fs::write(path, &yaml) {
                    Ok(_) => self.status_message = format!("Saved to {}", path.display()),
                    Err(error) => self.status_message = format!("Save error: {error}"),
                },
                Err(error) => self.status_message = format!("Serialize error: {error}"),
            }
        } else {
            self.status_message = "No file path — use Save As".into();
        }
    }

    /// Load a circuit from a YAML file, replacing the current state.
    pub fn load_circuit_from_yaml(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_yaml::from_str::<CircuitDef>(&contents) {
                Ok(loaded_circuit) => {
                    self.circuit = loaded_circuit;
                    self.file_path = Some(path.clone());
                    self.selected_component = None;
                    self.simulation_results = None;
                    self.simulation_error = None;
                    self.simulation_step = 0;
                    self.undo_stack = UndoStack::new(&self.circuit);
                    self.status_message = format!("Loaded {}", path.display());
                }
                Err(error) => self.status_message = format!("YAML parse error: {error}"),
            },
            Err(error) => self.status_message = format!("Read error: {error}"),
        }
    }

    /// Reset the editor to a blank circuit.
    pub fn new_circuit(&mut self) {
        self.circuit = CircuitDef {
            name: "untitled".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![],
        };
        self.file_path = None;
        self.selected_component = None;
        self.simulation_results = None;
        self.simulation_error = None;
        self.simulation_step = 0;
        self.undo_stack = UndoStack::new(&self.circuit);
        self.status_message = "New circuit".into();
    }

    /// Delete the currently selected component.
    pub fn delete_selected_component(&mut self) {
        if let Some(selected_index) = self.selected_component.take() {
            if selected_index < self.active_components().len() {
                self.save_undo_checkpoint();
                self.active_components_mut().remove(selected_index);
                self.status_message = "Component deleted".into();
            }
        }
    }

    /// Duplicate the currently selected component with an offset position.
    pub fn duplicate_selected_component(&mut self) {
        if let Some(selected_index) = self.selected_component {
            if selected_index < self.active_components().len() {
                self.save_undo_checkpoint();
                let mut duplicated = self.active_components()[selected_index].clone();
                duplicated.name = format!("{}_copy", duplicated.name);
                if let Some([x, y]) = duplicated.pos {
                    duplicated.pos = Some([x + COMPONENT_WIDTH + 20.0, y]);
                }
                self.active_components_mut().push(duplicated);
                self.selected_component = Some(self.active_components().len() - 1);
                self.status_message = "Component duplicated".into();
            }
        }
    }
}
