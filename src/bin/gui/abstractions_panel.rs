use eframe::egui;
use std::collections::HashMap;

use diycpu::types::AbstractionDef;

use super::app::App;

impl App {
    /// Draw the abstractions panel in the right sidebar.
    /// Contains the list of abstractions, details for the selected one,
    /// and a form to create new abstractions.
    pub fn draw_abstractions_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Abstractions");
        self.draw_abstraction_list(ui);
        self.draw_selected_abstraction_details(ui);
        ui.separator();
        self.draw_new_abstraction_form(ui);
    }

    fn draw_abstraction_list(&mut self, ui: &mut egui::Ui) {
        let abstraction_names: Vec<String> = self
            .circuit
            .abstractions
            .iter()
            .map(|abstraction| abstraction.name.clone())
            .collect();

        if abstraction_names.is_empty() {
            ui.label("No abstractions defined.");
        } else {
            for (index, name) in abstraction_names.iter().enumerate() {
                let is_selected = self.selected_abstraction_index == Some(index);
                if ui.selectable_label(is_selected, name).clicked() {
                    self.selected_abstraction_index = Some(index);
                }
            }
        }
    }

    fn draw_selected_abstraction_details(&mut self, ui: &mut egui::Ui) {
        let Some(selected_index) = self.selected_abstraction_index else {
            return;
        };
        if selected_index >= self.circuit.abstractions.len() {
            return;
        }

        ui.separator();
        let abstraction = &self.circuit.abstractions[selected_index];
        ui.strong(&abstraction.name);
        ui.label(format!("Inputs: {}", abstraction.inputs.join(", ")));
        ui.label("Outputs:");
        for (output_name, target_name) in &abstraction.outputs {
            ui.label(format!("  {output_name} -> {target_name}"));
        }
        ui.label(format!("Components: {}", abstraction.components.len()));
        for component in &abstraction.components {
            ui.label(format!(
                "  {} ({})",
                component.name, component.component_type
            ));
        }
        ui.add_space(4.0);
        if ui.button("Delete abstraction").clicked() {
            self.save_undo_checkpoint();
            self.circuit.abstractions.remove(selected_index);
            self.selected_abstraction_index = None;
            self.status_message = "Abstraction deleted".into();
        }
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
        ui.label("Output mapping:");
        ui.horizontal(|ui| {
            ui.label("  key:");
            ui.text_edit_singleline(&mut self.abstraction_editor_output_key);
            ui.label("val:");
            ui.text_edit_singleline(&mut self.abstraction_editor_output_value);
        });

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
            if !self.abstraction_editor_output_key.is_empty() {
                outputs.insert(
                    self.abstraction_editor_output_key.clone(),
                    self.abstraction_editor_output_value.clone(),
                );
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
            self.abstraction_editor_output_key.clear();
            self.abstraction_editor_output_value.clear();
            self.status_message = "Abstraction created".into();
        }
    }
}
