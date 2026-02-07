use eframe::egui;
use std::collections::HashMap;

use diycpu::types::{AbstractionDef, ComponentDef};

use super::app::App;
use super::style::{WIRE_HIGH, WIRE_LOW};

impl App {
    /// Draw the components panel in the right sidebar.
    /// Context-aware: adjusts heading and controls based on whether we're
    /// editing the main circuit or inside an abstraction.
    pub fn draw_components_panel(&mut self, ui: &mut egui::Ui) {
        if self.editing_abstraction_index.is_some() {
            self.draw_abstraction_context_header(ui);
        } else {
            self.draw_circuit_settings(ui);
        }
        ui.separator();
        self.draw_add_component_form(ui);
        ui.separator();
        self.draw_selected_component_details(ui);
    }

    fn draw_circuit_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Circuit");
        ui.horizontal(|ui| {
            ui.label("Name:");
            if ui
                .text_edit_singleline(&mut self.circuit.name)
                .lost_focus()
            {
                self.save_undo_checkpoint();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Steps:");
            if ui
                .add(egui::DragValue::new(&mut self.circuit.steps).range(1..=10000))
                .changed()
            {
                self.save_undo_checkpoint();
            }
        });
    }

    fn draw_abstraction_context_header(&mut self, ui: &mut egui::Ui) {
        let abs_idx = self.editing_abstraction_index.unwrap();
        if abs_idx >= self.circuit.abstractions.len() {
            return;
        }
        let abs_name = self.circuit.abstractions[abs_idx].name.clone();
        ui.heading(format!("Abstraction: {abs_name}"));

        // Show and edit abstraction inputs
        let inputs_str = self.circuit.abstractions[abs_idx].inputs.join(", ");
        ui.label(format!("Inputs: {inputs_str}"));

        // Show available input signals as a hint
        ui.label("(Use these names as wire sources inside)");
    }

    fn draw_add_component_form(&mut self, ui: &mut egui::Ui) {
        ui.heading("Add Component");
        let is_editing_abstraction = self.editing_abstraction_index.is_some();

        ui.horizontal(|ui| {
            ui.label("Type:");
            egui::ComboBox::from_id_salt("add_type")
                .selected_text(&self.new_component_type)
                .show_ui(ui, |ui| {
                    // CLK is forbidden inside abstractions
                    if !is_editing_abstraction {
                        ui.selectable_value(
                            &mut self.new_component_type,
                            "clk".into(),
                            "clk",
                        );
                    }
                    ui.selectable_value(&mut self.new_component_type, "nand".into(), "nand");
                    ui.selectable_value(&mut self.new_component_type, "dff".into(), "dff");
                    for abstraction in &self.circuit.abstractions {
                        let name = abstraction.name.clone();
                        ui.selectable_value(
                            &mut self.new_component_type,
                            name.clone(),
                            &name,
                        );
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.new_component_name);
        });
        if ui.button("Add").clicked() && !self.new_component_name.is_empty() {
            self.save_undo_checkpoint();
            let default_inputs =
                make_default_inputs(&self.new_component_type, &self.circuit.abstractions);
            let new_component = ComponentDef {
                component_type: self.new_component_type.clone(),
                name: self.new_component_name.clone(),
                init: if self.new_component_type == "dff" {
                    Some(false)
                } else {
                    None
                },
                inputs: default_inputs,
                pos: Some([80.0, 80.0]),
                wire_waypoints: None,
            };
            self.active_components_mut().push(new_component);
            self.selected_component = Some(self.active_components().len() - 1);
            self.new_component_name.clear();
            self.status_message = "Component added".into();
        }
    }

    fn draw_selected_component_details(&mut self, ui: &mut egui::Ui) {
        let Some(selected_index) = self.selected_component else {
            ui.label("Click a component to select it.");
            ui.label("Click an output port to start wiring.");
            return;
        };

        if selected_index >= self.active_components().len() {
            self.selected_component = None;
            return;
        }

        ui.heading("Selected Component");

        // Editable type via ComboBox
        let is_editing_abstraction = self.editing_abstraction_index.is_some();
        let current_type = self.active_components()[selected_index]
            .component_type
            .clone();
        let mut new_type = current_type.clone();
        ui.horizontal(|ui| {
            ui.label("Type:");
            egui::ComboBox::from_id_salt("edit_type")
                .selected_text(&new_type)
                .show_ui(ui, |ui| {
                    if !is_editing_abstraction {
                        ui.selectable_value(&mut new_type, "clk".into(), "clk");
                    }
                    ui.selectable_value(&mut new_type, "nand".into(), "nand");
                    ui.selectable_value(&mut new_type, "dff".into(), "dff");
                    for abstraction in &self.circuit.abstractions {
                        let name = abstraction.name.clone();
                        ui.selectable_value(&mut new_type, name.clone(), &name);
                    }
                });
        });
        if new_type != current_type {
            self.save_undo_checkpoint();
            let default_inputs =
                make_default_inputs(&new_type, &self.circuit.abstractions);
            let comp = &mut self.active_components_mut()[selected_index];
            comp.component_type = new_type.clone();
            comp.inputs = default_inputs;
            if new_type == "dff" {
                comp.init = Some(false);
            } else {
                comp.init = None;
            }
        }

        let component = &mut self.active_components_mut()[selected_index];
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut component.name);
        });

        // DFF init value checkbox
        if component.component_type == "dff" {
            let mut init_value = component.init.unwrap_or(false);
            if ui.checkbox(&mut init_value, "Init value").changed() {
                component.init = Some(init_value);
            }
        }

        // Input connection editors
        let mut input_to_remove = None;
        if let Some(ref mut inputs) = component.inputs {
            ui.label("Inputs:");
            let sorted_keys: Vec<String> = {
                let mut keys: Vec<_> = inputs.keys().cloned().collect();
                keys.sort();
                keys
            };
            for key in &sorted_keys {
                ui.horizontal(|ui| {
                    ui.label(format!("  {key}:"));
                    let value = inputs.get_mut(key).unwrap();
                    ui.add(egui::TextEdit::singleline(value).desired_width(120.0));
                    if ui.small_button("X").clicked() {
                        input_to_remove = Some(key.clone());
                    }
                });
            }
        }

        // Remove input if X was clicked
        if let Some(key) = input_to_remove {
            self.save_undo_checkpoint();
            if let Some(ref mut inputs) = self.active_components_mut()[selected_index].inputs {
                inputs.remove(&key);
            }
        }

        // Add new input key
        ui.horizontal(|ui| {
            ui.label("  Add input:");
            // Use a static buffer stored per-frame via egui memory
            let mut new_key = String::new();
            let response = ui.add(
                egui::TextEdit::singleline(&mut new_key)
                    .desired_width(80.0)
                    .hint_text("key name"),
            );
            if response.lost_focus()
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && !new_key.is_empty()
            {
                self.save_undo_checkpoint();
                let comp = &mut self.active_components_mut()[selected_index];
                let inputs = comp.inputs.get_or_insert_with(HashMap::new);
                inputs.insert(new_key, String::new());
            }
        });

        // Show simulation value for the selected component
        let component_name = self.active_components()[selected_index].name.clone();
        if let Some(value) = self.simulation_value(&component_name) {
            let (display_text, display_color) = if value {
                ("= 1", WIRE_HIGH)
            } else {
                ("= 0", WIRE_LOW)
            };
            ui.colored_label(display_color, display_text);
        }

        // Position display
        if let Some([x, y]) = &self.active_components()[selected_index].pos {
            ui.label(format!("Pos: ({x:.0}, {y:.0})"));
        }

        // Action buttons
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Delete").clicked() {
                self.delete_selected_component();
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate_selected_component();
            }
        });
    }
}

/// Create a default input map for a new component based on its type.
/// Primitives get their standard inputs; abstraction instances get inputs
/// matching the abstraction's declared input list.
fn make_default_inputs(
    component_type: &str,
    abstractions: &[AbstractionDef],
) -> Option<HashMap<String, String>> {
    match component_type {
        "clk" => None,
        "nand" => {
            let mut inputs = HashMap::new();
            inputs.insert("a".into(), String::new());
            inputs.insert("b".into(), String::new());
            Some(inputs)
        }
        "dff" => {
            let mut inputs = HashMap::new();
            inputs.insert("d".into(), String::new());
            inputs.insert("clk".into(), String::new());
            Some(inputs)
        }
        _ => {
            let matching_abstraction = abstractions.iter().find(|a| a.name == component_type);
            if let Some(abstraction) = matching_abstraction {
                let mut inputs = HashMap::new();
                for input_name in &abstraction.inputs {
                    inputs.insert(input_name.clone(), String::new());
                }
                Some(inputs)
            } else {
                None
            }
        }
    }
}
