use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use diycpu::compiler::compile;
use diycpu::convert::project_to_circuit;
use diycpu::expander::expand;
use diycpu::project::*;
use diycpu::simulator::simulate;

use super::style::{component_size, COMPONENT_WIDTH};
use super::undo::UndoStack;

// ── Interaction mode state machine ──────────────────────────────────────────

#[derive(Clone)]
pub enum InteractionMode {
    /// Default mode: click to select, drag to move.
    Normal,
    /// Drawing a wire from an output port or junction.
    DrawingWire {
        net_id: WireNetId,
        from_endpoint: Endpoint,
    },
    /// Box-selecting components by dragging a rectangle.
    BoxSelecting {
        start_pos: egui::Pos2,
    },
    /// Dragging one or more selected components (and junctions).
    DraggingComponents {
        original_positions: HashMap<ComponentId, [f32; 2]>,
        original_junction_positions: HashMap<(WireNetId, JunctionId), [f32; 2]>,
        drag_start_world: [f32; 2],
    },
    /// Dragging a junction point on a wire.
    DraggingJunction {
        net_id: WireNetId,
        junction_id: JunctionId,
    },
}

impl Default for InteractionMode {
    fn default() -> Self {
        Self::Normal
    }
}

// ── Panel tabs ──────────────────────────────────────────────────────────────

/// Which tab is currently active in the right side panel.
#[derive(PartialEq, Clone, Copy)]
pub enum PanelTab {
    Components,
    Abstractions,
}

// ── Main application state ──────────────────────────────────────────────────

pub struct App {
    /// The project being edited.
    pub project: Project,
    /// Path to the currently open file (if any).
    pub file_path: Option<PathBuf>,

    /// Set of currently selected component IDs.
    pub selected_components: HashSet<ComponentId>,
    /// Set of currently selected junctions: (net_id, junction_id).
    pub selected_junctions: HashSet<(WireNetId, JunctionId)>,
    /// Current interaction mode.
    pub interaction_mode: InteractionMode,

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

    /// Simulation results (one HashMap per step), or None if no simulation has run.
    pub simulation_results: Option<Vec<HashMap<String, bool>>>,
    /// The currently displayed simulation step index.
    pub simulation_step: usize,
    /// Error message from the last simulation attempt, if any.
    pub simulation_error: Option<String>,

    /// Status bar message shown at the top right of the menu bar.
    pub status_message: String,

    /// Counter for generating unique IDs.
    pub next_id: u32,
}

impl Default for App {
    fn default() -> Self {
        let project = Project {
            name: "untitled".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![],
            wire_nets: vec![],
        };
        Self {
            undo_stack: UndoStack::new(&project),
            project,
            file_path: None,
            selected_components: HashSet::new(),
            selected_junctions: HashSet::new(),
            interaction_mode: InteractionMode::Normal,
            canvas_offset: egui::Vec2::ZERO,
            zoom: 1.0,
            is_panning: false,
            last_pan_pointer_position: None,
            new_component_type: "nand".into(),
            active_panel_tab: PanelTab::Components,
            abstraction_editor_name: String::new(),
            abstraction_editor_inputs: String::new(),
            abstraction_editor_outputs: vec![("out".into(), String::new())],
            selected_abstraction_index: None,
            editing_abstraction_index: None,
            simulation_results: None,
            simulation_step: 0,
            simulation_error: None,
            status_message: "Ready — scroll to zoom, right-drag to pan".into(),
            next_id: 0,
        }
    }
}

// ── Core methods ────────────────────────────────────────────────────────────

impl App {
    /// Clear both component and junction selections.
    pub fn clear_selection(&mut self) {
        self.selected_components.clear();
        self.selected_junctions.clear();
    }

    /// Returns a reference to the components currently being edited.
    pub fn active_components(&self) -> &[ProjectComponent] {
        if let Some(abs_idx) = self.editing_abstraction_index {
            &self.project.abstractions[abs_idx].components
        } else {
            &self.project.components
        }
    }

    /// Returns a mutable reference to the components currently being edited.
    pub fn active_components_mut(&mut self) -> &mut Vec<ProjectComponent> {
        if let Some(abs_idx) = self.editing_abstraction_index {
            &mut self.project.abstractions[abs_idx].components
        } else {
            &mut self.project.components
        }
    }

    /// Returns a reference to the wire nets currently being edited.
    pub fn active_wire_nets(&self) -> &[WireNet] {
        if let Some(abs_idx) = self.editing_abstraction_index {
            &self.project.abstractions[abs_idx].wire_nets
        } else {
            &self.project.wire_nets
        }
    }

    /// Returns a mutable reference to the wire nets currently being edited.
    pub fn active_wire_nets_mut(&mut self) -> &mut Vec<WireNet> {
        if let Some(abs_idx) = self.editing_abstraction_index {
            &mut self.project.abstractions[abs_idx].wire_nets
        } else {
            &mut self.project.wire_nets
        }
    }

    /// Returns the abstraction being edited, if any.
    pub fn editing_abstraction(&self) -> Option<&ProjectAbstraction> {
        self.editing_abstraction_index
            .and_then(|idx| self.project.abstractions.get(idx))
    }

    /// Save the current project state to the undo stack.
    pub fn save_undo_checkpoint(&mut self) {
        self.undo_stack.push(&self.project);
    }

    /// Undo the last change, restoring the previous project state.
    pub fn undo_last_change(&mut self) {
        if let Some(previous_state) = self.undo_stack.undo() {
            self.project = previous_state.clone();
            self.clear_selection();
            self.status_message = "Undo".into();
        }
    }

    /// Redo the last undone change.
    pub fn redo_last_change(&mut self) {
        if let Some(next_state) = self.undo_stack.redo() {
            self.project = next_state.clone();
            self.clear_selection();
            self.status_message = "Redo".into();
        }
    }

    /// Generate a unique component ID with the given type prefix.
    pub fn generate_id(&mut self, prefix: &str) -> String {
        let existing_ids: Vec<String> =
            self.active_components().iter().map(|c| c.id.clone()).collect();
        loop {
            self.next_id += 1;
            let id = format!("{prefix}_{}", self.next_id);
            if !existing_ids.contains(&id) {
                return id;
            }
        }
    }

    /// Generate a unique wire net ID.
    pub fn generate_wire_net_id(&mut self) -> String {
        let existing_ids: Vec<String> =
            self.active_wire_nets().iter().map(|n| n.id.clone()).collect();
        loop {
            self.next_id += 1;
            let id = format!("w_{}", self.next_id);
            if !existing_ids.contains(&id) {
                return id;
            }
        }
    }

    /// Generate a unique junction ID.
    pub fn generate_junction_id(&mut self) -> String {
        self.next_id += 1;
        format!("j_{}", self.next_id)
    }

    /// Compute the screen-space rectangle for a component.
    pub fn component_screen_rect(
        &self,
        comp: &ProjectComponent,
        canvas_origin: egui::Pos2,
    ) -> egui::Rect {
        let pos = egui::pos2(comp.pos[0], comp.pos[1]);
        let size = component_size(&comp.component_type, &self.project.abstractions);
        let screen_pos = canvas_origin + pos.to_vec2() * self.zoom;
        egui::Rect::from_min_size(screen_pos, size * self.zoom)
    }

    /// Run the simulation and store the results.
    pub fn run_simulation(&mut self) {
        self.simulation_results = None;
        self.simulation_error = None;
        self.simulation_step = 0;

        let circuit_def = match project_to_circuit(&self.project) {
            Ok(def) => def,
            Err(e) => {
                self.simulation_error = Some(format!("Convert: {e}"));
                return;
            }
        };

        let expanded = match expand(&circuit_def) {
            Ok(e) => e,
            Err(e) => {
                self.simulation_error = Some(format!("Expand: {e}"));
                return;
            }
        };

        let compiled = match compile(&expanded.def) {
            Ok(c) => c,
            Err(e) => {
                self.simulation_error = Some(format!("Compile: {e}"));
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
        self.status_message = format!("Simulated {} steps", self.project.steps);
    }

    /// Get the value of a component at the current simulation step.
    pub fn simulation_value(&self, component_id: &str) -> Option<bool> {
        self.simulation_results
            .as_ref()
            .and_then(|results| results.get(self.simulation_step))
            .and_then(|step_values| step_values.get(component_id).copied())
    }

    /// Save the project to the current file path as JSON.
    pub fn save_to_json(&mut self) {
        if let Some(path) = &self.file_path {
            match serde_json::to_string_pretty(&self.project) {
                Ok(json) => match std::fs::write(path, &json) {
                    Ok(_) => self.status_message = format!("Saved to {}", path.display()),
                    Err(e) => self.status_message = format!("Save error: {e}"),
                },
                Err(e) => self.status_message = format!("Serialize error: {e}"),
            }
        } else {
            self.status_message = "No file path — use Save As".into();
        }
    }

    /// Load a project from a JSON file, replacing the current state.
    pub fn load_from_json(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<Project>(&contents) {
                Ok(loaded) => {
                    self.project = loaded;
                    self.file_path = Some(path.clone());
                    self.clear_selection();
                    self.interaction_mode = InteractionMode::Normal;
                    self.simulation_results = None;
                    self.simulation_error = None;
                    self.simulation_step = 0;
                    self.undo_stack = UndoStack::new(&self.project);
                    self.status_message = format!("Loaded {}", path.display());
                }
                Err(e) => self.status_message = format!("JSON parse error: {e}"),
            },
            Err(e) => self.status_message = format!("Read error: {e}"),
        }
    }

    /// Reset the editor to a blank project.
    pub fn new_circuit(&mut self) {
        self.project = Project {
            name: "untitled".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![],
            wire_nets: vec![],
        };
        self.file_path = None;
        self.clear_selection();
        self.interaction_mode = InteractionMode::Normal;
        self.simulation_results = None;
        self.simulation_error = None;
        self.simulation_step = 0;
        self.undo_stack = UndoStack::new(&self.project);
        self.next_id = 0;
        self.editing_abstraction_index = None;
        self.status_message = "New circuit".into();
    }

    /// Delete all currently selected components and junctions, and clean up wire nets.
    pub fn delete_selected_components(&mut self) {
        if self.selected_components.is_empty() && self.selected_junctions.is_empty() {
            return;
        }
        self.save_undo_checkpoint();

        let to_delete = self.selected_components.clone();
        let junctions_to_delete = self.selected_junctions.clone();

        // Remove components
        if !to_delete.is_empty() {
            self.active_components_mut()
                .retain(|c| !to_delete.contains(&c.id));
        }

        // Clean up wire nets
        for net in self.active_wire_nets_mut().iter_mut() {
            // Remove sinks referencing deleted components
            net.sinks
                .retain(|s| !to_delete.contains(&s.component_id));

            // Remove segments referencing deleted component ports
            net.segments.retain(|seg| {
                !endpoint_references_any(&seg.from, &to_delete)
                    && !endpoint_references_any(&seg.to, &to_delete)
            });

            // Clear source if it references a deleted component
            if let Some(ref source) = net.source {
                if to_delete.contains(&source.component_id) {
                    net.source = None;
                }
            }

            // Delete selected junctions from this net
            for (jnet_id, jid) in &junctions_to_delete {
                if *jnet_id == net.id {
                    // Remove segments connected to this junction
                    net.segments.retain(|seg| {
                        !matches!(&seg.from, Endpoint::Junction { junction_id } if junction_id == jid)
                            && !matches!(&seg.to, Endpoint::Junction { junction_id } if junction_id == jid)
                    });
                    // Remove the junction itself
                    net.junctions.retain(|j| j.id != *jid);
                }
            }
        }

        // Remove nets that have no source and no sinks
        self.active_wire_nets_mut()
            .retain(|net| net.source.is_some() || !net.sinks.is_empty());

        self.clear_selection();
        self.status_message = "Deleted".into();
    }

    /// Duplicate the currently selected component (single selection only).
    pub fn duplicate_selected_component(&mut self) {
        if self.selected_components.len() != 1 {
            return;
        }
        let id = self.selected_components.iter().next().unwrap().clone();

        let comp_idx = self.active_components().iter().position(|c| c.id == id);
        let Some(idx) = comp_idx else { return };

        self.save_undo_checkpoint();
        let original = self.active_components()[idx].clone();
        let new_id = self.generate_id(&original.component_type);
        let new_comp = ProjectComponent {
            id: new_id.clone(),
            component_type: original.component_type,
            init: original.init,
            pos: [original.pos[0] + COMPONENT_WIDTH + 20.0, original.pos[1]],
        };
        self.active_components_mut().push(new_comp);
        self.clear_selection();
        self.selected_components.insert(new_id);
        self.status_message = "Component duplicated".into();
    }

    /// Cancel wire drawing. If the net is brand-new (started from a port, no segments placed),
    /// remove it entirely. Otherwise, just stop drawing and keep existing segments/junctions.
    pub fn cancel_wire_drawing(&mut self) {
        if let InteractionMode::DrawingWire {
            ref net_id,
            ref from_endpoint,
        } = self.interaction_mode
        {
            let net_id = net_id.clone();
            let is_from_port = matches!(from_endpoint, Endpoint::Port { .. });

            // Check if the net has any segments
            let net_has_segments = self
                .active_wire_nets()
                .iter()
                .find(|n| n.id == net_id)
                .map(|n| !n.segments.is_empty())
                .unwrap_or(false);

            if is_from_port && !net_has_segments {
                // Brand-new empty net — remove it entirely
                self.active_wire_nets_mut().retain(|n| n.id != net_id);
                self.status_message = "Wire cancelled".into();
            } else {
                // Net has segments/junctions — keep them, just stop drawing
                self.status_message = "Wire drawing stopped".into();
            }
        }
        self.interaction_mode = InteractionMode::Normal;
    }

    /// Disconnect a specific input port on a component by removing the sink from its wire net.
    pub fn disconnect_port(&mut self, component_id: &str, port_name: &str) {
        self.save_undo_checkpoint();
        for net in self.active_wire_nets_mut().iter_mut() {
            net.sinks
                .retain(|s| !(s.component_id == component_id && s.port_name == port_name));

            // Remove segments that reference this port
            net.segments.retain(|seg| {
                !endpoint_matches_port(&seg.from, component_id, port_name)
                    && !endpoint_matches_port(&seg.to, component_id, port_name)
            });
        }

        // Clean up empty nets
        self.active_wire_nets_mut()
            .retain(|net| net.source.is_some() || !net.sinks.is_empty());
    }

    /// Find connections to a component's input ports from wire nets.
    /// Returns (port_name, source_description) pairs.
    pub fn connections_for_component(&self, component_id: &str) -> Vec<(String, String)> {
        let mut connections = Vec::new();
        for net in self.active_wire_nets() {
            for sink in &net.sinks {
                if sink.component_id == component_id {
                    let source_desc = if let Some(ref source) = net.source {
                        if source.port_name == "out" {
                            source.component_id.clone()
                        } else {
                            format!("{}.{}", source.component_id, source.port_name)
                        }
                    } else {
                        "unconnected".into()
                    };
                    connections.push((sink.port_name.clone(), source_desc));
                }
            }
        }
        connections.sort();
        connections
    }
}

/// Check if an endpoint references any of the given component IDs.
fn endpoint_references_any(endpoint: &Endpoint, component_ids: &HashSet<ComponentId>) -> bool {
    match endpoint {
        Endpoint::Port { component_id, .. } => component_ids.contains(component_id),
        Endpoint::Junction { .. } => false,
    }
}

/// Check if an endpoint matches a specific component port.
fn endpoint_matches_port(endpoint: &Endpoint, comp_id: &str, port: &str) -> bool {
    matches!(
        endpoint,
        Endpoint::Port { component_id, port_name }
            if component_id == comp_id && port_name == port
    )
}
