use eframe::egui;

use diycpu::project::ProjectComponent;

use super::app::App;
use super::style::{input_port_names, WIRE_HIGH, WIRE_LOW};

impl App {
    /// Draw the components panel in the right sidebar.
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
                .text_edit_singleline(&mut self.project.name)
                .lost_focus()
            {
                self.save_undo_checkpoint();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Steps:");
            if ui
                .add(egui::DragValue::new(&mut self.project.steps).range(1..=10000))
                .changed()
            {
                self.save_undo_checkpoint();
            }
        });
    }

    fn draw_abstraction_context_header(&mut self, ui: &mut egui::Ui) {
        let abs_idx = self.editing_abstraction_index.unwrap();
        if abs_idx >= self.project.abstractions.len() {
            return;
        }
        let abs_name = self.project.abstractions[abs_idx].name.clone();
        ui.heading(format!("Abstraction: {abs_name}"));

        let inputs_str = self.project.abstractions[abs_idx].inputs.join(", ");
        ui.label(format!("Inputs: {inputs_str}"));
        ui.label("(Wire from input pins on the left edge)");
    }

    fn draw_add_component_form(&mut self, ui: &mut egui::Ui) {
        ui.heading("Add Component");
        let is_editing_abstraction = self.editing_abstraction_index.is_some();

        ui.horizontal(|ui| {
            ui.label("Type:");
            egui::ComboBox::from_id_salt("add_type")
                .selected_text(&self.new_component_type)
                .show_ui(ui, |ui| {
                    if !is_editing_abstraction {
                        ui.selectable_value(&mut self.new_component_type, "clk".into(), "clk");
                    }
                    ui.selectable_value(&mut self.new_component_type, "nand".into(), "nand");
                    ui.selectable_value(&mut self.new_component_type, "dff".into(), "dff");
                    for abstraction in &self.project.abstractions {
                        let name = abstraction.name.clone();
                        ui.selectable_value(
                            &mut self.new_component_type,
                            name.clone(),
                            &name,
                        );
                    }
                });
        });

        if ui.button("Add").clicked() {
            self.save_undo_checkpoint();
            let new_id = self.generate_id(&self.new_component_type.clone());
            let new_component = ProjectComponent {
                id: new_id.clone(),
                component_type: self.new_component_type.clone(),
                init: if self.new_component_type == "dff" {
                    Some(false)
                } else {
                    None
                },
                pos: [80.0, 80.0],
            };
            self.active_components_mut().push(new_component);
            self.clear_selection();
            self.selected_components.insert(new_id);
            self.status_message = "Component added".into();
        }
    }

    fn draw_selected_component_details(&mut self, ui: &mut egui::Ui) {
        if self.selected_components.is_empty() {
            ui.label("Click a component to select it.");
            ui.label("Click an output port to start wiring.");
            return;
        }

        if self.selected_components.len() > 1 {
            ui.heading(format!("{} components selected", self.selected_components.len()));
            ui.add_space(4.0);
            if ui.button("Delete all").clicked() {
                self.delete_selected_components();
            }
            return;
        }

        // Single selection
        let selected_id = self.selected_components.iter().next().unwrap().clone();
        let comp_idx = self
            .active_components()
            .iter()
            .position(|c| c.id == selected_id);
        let Some(idx) = comp_idx else {
            self.clear_selection();
            return;
        };

        ui.heading("Selected Component");

        // Editable type via ComboBox
        let is_editing_abstraction = self.editing_abstraction_index.is_some();
        let current_type = self.active_components()[idx].component_type.clone();
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
                    for abstraction in &self.project.abstractions {
                        let name = abstraction.name.clone();
                        ui.selectable_value(&mut new_type, name.clone(), &name);
                    }
                });
        });
        if new_type != current_type {
            self.save_undo_checkpoint();
            let comp = &mut self.active_components_mut()[idx];
            comp.component_type = new_type.clone();
            if new_type == "dff" {
                comp.init = Some(false);
            } else {
                comp.init = None;
            }
        }

        // Editable ID
        let component = &mut self.active_components_mut()[idx];
        ui.horizontal(|ui| {
            ui.label("ID:");
            ui.text_edit_singleline(&mut component.id);
        });

        // DFF init value checkbox
        if component.component_type == "dff" {
            let mut init_value = component.init.unwrap_or(false);
            if ui.checkbox(&mut init_value, "Init value").changed() {
                component.init = Some(init_value);
            }
        }

        // Connected wires (read-only with disconnect buttons)
        let comp_id = self.active_components()[idx].id.clone();
        let comp_type = self.active_components()[idx].component_type.clone();
        let port_names = input_port_names(&comp_type, &self.project.abstractions);
        let connections = self.connections_for_component(&comp_id);

        if !port_names.is_empty() {
            ui.label("Connections:");
            let mut port_to_disconnect = None;
            for port_name in &port_names {
                let source = connections
                    .iter()
                    .find(|(p, _)| p == port_name)
                    .map(|(_, s)| s.as_str())
                    .unwrap_or("—");
                ui.horizontal(|ui| {
                    ui.label(format!("  {port_name} <-"));
                    ui.label(source);
                    if source != "—" && ui.small_button("X").clicked() {
                        port_to_disconnect = Some(port_name.clone());
                    }
                });
            }
            if let Some(port) = port_to_disconnect {
                self.disconnect_port(&comp_id, &port);
            }
        }

        // Show simulation value
        if let Some(value) = self.simulation_value(&comp_id) {
            let (display_text, display_color) = if value {
                ("= 1", WIRE_HIGH)
            } else {
                ("= 0", WIRE_LOW)
            };
            ui.colored_label(display_color, display_text);
        }

        // Position display
        let [x, y] = self.active_components()[idx].pos;
        ui.label(format!("Pos: ({x:.0}, {y:.0})"));

        // Action buttons
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Delete").clicked() {
                self.delete_selected_components();
            }
            if ui.button("Duplicate").clicked() {
                self.duplicate_selected_component();
            }
        });
    }
}
