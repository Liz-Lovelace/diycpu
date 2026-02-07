use eframe::egui;

use super::app::App;
use super::style::{WIRE_HIGH, WIRE_LOW};

impl App {
    /// Draw the simulation playback bar at the bottom of the screen.
    /// Only shown when simulation results are available.
    pub fn draw_simulation_playback_bar(&mut self, ctx: &egui::Context) {
        if self.simulation_results.is_none() {
            return;
        }

        egui::TopBottomPanel::bottom("sim_playback")
            .min_height(36.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let max_step = self
                        .simulation_results
                        .as_ref()
                        .map(|results| results.len().saturating_sub(1))
                        .unwrap_or(0);

                    ui.label("Step:");
                    ui.add(
                        egui::Slider::new(&mut self.simulation_step, 0..=max_step)
                            .text(format!("/ {max_step}")),
                    );

                    // Transport buttons
                    if ui.button("|<").clicked() {
                        self.simulation_step = 0;
                    }
                    if ui.button("<").clicked() {
                        self.simulation_step = self.simulation_step.saturating_sub(1);
                    }
                    if ui.button(">").clicked() {
                        if self.simulation_step < max_step {
                            self.simulation_step += 1;
                        }
                    }
                    if ui.button(">|").clicked() {
                        self.simulation_step = max_step;
                    }
                    if ui.button("Clear sim").clicked() {
                        self.simulation_results = None;
                        self.simulation_step = 0;
                    }
                });
            });
    }

    /// Draw the simulation results table in the right side panel.
    /// Shows signal values for the current step.
    pub fn draw_simulation_results_table(&mut self, ui: &mut egui::Ui) {
        let has_error = self.simulation_error.is_some();
        let has_results = self.simulation_results.is_some();

        if !has_error && !has_results {
            return;
        }

        ui.separator();
        ui.heading("Simulation");

        if let Some(ref error_message) = self.simulation_error {
            ui.colored_label(egui::Color32::RED, error_message);
        }

        if let Some(ref results) = self.simulation_results {
            if let Some(step_values) = results.get(self.simulation_step) {
                let mut sorted_entries: Vec<_> = step_values.iter().collect();
                sorted_entries.sort_by_key(|(name, _)| (*name).clone());

                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        egui::Grid::new("sim_values")
                            .striped(true)
                            .show(ui, |ui| {
                                ui.strong("Signal");
                                ui.strong("Value");
                                ui.end_row();

                                for (signal_name, &signal_value) in &sorted_entries {
                                    // Skip internal expanded names (contain '/')
                                    if signal_name.contains('/') {
                                        continue;
                                    }
                                    ui.label(*signal_name);
                                    let (display_text, display_color) = if signal_value {
                                        ("1", WIRE_HIGH)
                                    } else {
                                        ("0", WIRE_LOW)
                                    };
                                    ui.colored_label(display_color, display_text);
                                    ui.end_row();
                                }
                            });
                    });
            }
        }
    }
}
