use eframe::egui;
use std::collections::HashMap;

use diycpu::types::AbstractionDef;

use super::app::{App, PanelTab};

impl App {
    /// Draw the abstractions panel in the right sidebar.
    pub fn draw_abstractions_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Abstractions");
        self.draw_abstraction_list(ui);
        self.draw_selected_abstraction_details(ui);
        ui.separator();
        self.draw_new_abstraction_form(ui);
    }

    fn draw_abstraction_list(&mut self, ui: &mut egui::Ui) {
        let abstraction_count = self.circuit.abstractions.len();

        if abstraction_count == 0 {
            ui.label("No abstractions defined.");
        } else {
            for index in 0..abstraction_count {
                let name = self.circuit.abstractions[index].name.clone();
                let comp_count = self.circuit.abstractions[index].components.len();
                let label = format!("{name} ({comp_count} components)");
                ui.horizontal(|ui| {
                    let is_selected = self.selected_abstraction_index == Some(index);
                    if ui.selectable_label(is_selected, &label).clicked() {
                        self.selected_abstraction_index = Some(index);
                    }
                    if ui.small_button("Edit").clicked() {
                        self.editing_abstraction_index = Some(index);
                        self.selected_component = None;
                        self.canvas_offset = egui::Vec2::ZERO;
                        self.active_panel_tab = PanelTab::Components;
                        self.status_message = format!("Editing abstraction: {name}");
                    }
                });
            }
        }
    }

    fn draw_selected_abstraction_details(&mut self, ui: &mut egui::Ui) {
        let Some(selected_index) = self.selected_abstraction_index else {
            return;
        };
        if selected_index >= self.circuit.abstractions.len() {
            self.selected_abstraction_index = None;
            return;
        }

        ui.separator();

        // Editable name
        ui.horizontal(|ui| {
            ui.label("Name:");
            let name = &mut self.circuit.abstractions[selected_index].name;
            ui.text_edit_singleline(name);
        });

        // Editable inputs (comma-separated)
        let inputs_str = self.circuit.abstractions[selected_index].inputs.join(", ");
        let mut editable_inputs = inputs_str;
        ui.horizontal(|ui| {
            ui.label("Inputs:");
            if ui
                .text_edit_singleline(&mut editable_inputs)
                .lost_focus()
            {
                let parsed: Vec<String> = editable_inputs
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                self.save_undo_checkpoint();
                self.circuit.abstractions[selected_index].inputs = parsed;
            }
        });

        // Editable outputs
        ui.label("Outputs:");
        let outputs: Vec<(String, String)> = {
            let mut pairs: Vec<_> = self.circuit.abstractions[selected_index]
                .outputs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            pairs
        };
        let mut output_to_remove = None;
        for (key, value) in &outputs {
            ui.horizontal(|ui| {
                ui.label(format!("  {key} ->"));
                ui.label(value);
                if ui.small_button("X").clicked() {
                    output_to_remove = Some(key.clone());
                }
            });
        }
        if let Some(key) = output_to_remove {
            self.save_undo_checkpoint();
            self.circuit.abstractions[selected_index]
                .outputs
                .remove(&key);
        }

        // Add output pair
        ui.horizontal(|ui| {
            ui.label("  Add output:");
            let mut new_key = String::new();
            let mut new_val = String::new();
            ui.add(
                egui::TextEdit::singleline(&mut new_key)
                    .desired_width(50.0)
                    .hint_text("key"),
            );
            ui.label("->");
            ui.add(
                egui::TextEdit::singleline(&mut new_val)
                    .desired_width(50.0)
                    .hint_text("target"),
            );
            // Note: adding outputs is more practical via the main outputs editor below
        });

        // Components list
        ui.label(format!(
            "Components: {}",
            self.circuit.abstractions[selected_index].components.len()
        ));
        for component in &self.circuit.abstractions[selected_index].components {
            ui.label(format!(
                "  {} ({})",
                component.name, component.component_type
            ));
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Edit contents").clicked() {
                self.editing_abstraction_index = Some(selected_index);
                self.selected_component = None;
                self.canvas_offset = egui::Vec2::ZERO;
                self.active_panel_tab = PanelTab::Components;
                let name = self.circuit.abstractions[selected_index].name.clone();
                self.status_message = format!("Editing abstraction: {name}");
            }
            if ui.button("Delete").clicked() {
                self.save_undo_checkpoint();
                // If currently editing this abstraction, exit first
                if self.editing_abstraction_index == Some(selected_index) {
                    self.editing_abstraction_index = None;
                }
                self.circuit.abstractions.remove(selected_index);
                self.selected_abstraction_index = None;
                self.status_message = "Abstraction deleted".into();
            }
        });
    }

    fn draw_new_abstraction_form(&mut self, ui: &mut egui::Ui) {
        ui.heading("New Abstraction");
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut self.abstraction_editor_name);
        });
        ui.horizontal(|ui| {
            ui.label("Inputs (comma-separated):");
            ui.text_edit_singleline(&mut self.abstraction_editor_inputs);
        });

        ui.label("Outputs:");
        let mut remove_index = None;
        for (i, (key, val)) in self.abstraction_editor_outputs.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("  key:");
                ui.add(egui::TextEdit::singleline(key).desired_width(60.0));
                ui.label("->");
                ui.add(egui::TextEdit::singleline(val).desired_width(60.0));
                if ui.small_button("X").clicked() {
                    remove_index = Some(i);
                }
            });
        }
        if let Some(idx) = remove_index {
            self.abstraction_editor_outputs.remove(idx);
        }
        if ui.small_button("+ Add output").clicked() {
            self.abstraction_editor_outputs
                .push((String::new(), String::new()));
        }

        if ui.button("Create abstraction").clicked()
            && !self.abstraction_editor_name.is_empty()
        {
            self.save_undo_checkpoint();
            let parsed_inputs: Vec<String> = self
                .abstraction_editor_inputs
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let mut outputs = HashMap::new();
            for (key, val) in &self.abstraction_editor_outputs {
                if !key.is_empty() {
                    outputs.insert(key.clone(), val.clone());
                }
            }

            self.circuit.abstractions.push(AbstractionDef {
                name: self.abstraction_editor_name.clone(),
                inputs: parsed_inputs,
                outputs,
                components: vec![],
            });
            self.selected_abstraction_index = Some(self.circuit.abstractions.len() - 1);
            self.abstraction_editor_name.clear();
            self.abstraction_editor_inputs.clear();
            self.abstraction_editor_outputs = vec![("out".into(), String::new())];
            self.status_message = "Abstraction created".into();
        }
    }
}
