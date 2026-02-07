mod abstractions_panel;
mod app;
mod canvas;
mod components_panel;
mod file_dialogs;
mod simulation;
mod style;
mod undo;

use eframe::egui;

use app::{App, PanelTab};
use file_dialogs::{open_file_dialog, save_file_dialog};

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

// ── eframe::App implementation ──────────────────────────────────────────────

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        handle_keyboard_shortcuts(self, ctx);
        draw_top_menu_bar(self, ctx);
        draw_breadcrumb_bar(self, ctx);
        self.draw_simulation_playback_bar(ctx);
        handle_escape_cancel_wire(self, ctx);
        draw_right_side_panel(self, ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_canvas(ui, ctx);
        });
    }
}

// ── Keyboard shortcuts ──────────────────────────────────────────────────────

fn handle_keyboard_shortcuts(app: &mut App, ctx: &egui::Context) {
    let modifiers = ctx.input(|i| i.modifiers);
    let is_ctrl_held = modifiers.ctrl || modifiers.mac_cmd;

    if is_ctrl_held && ctx.input(|i| i.key_pressed(egui::Key::S)) {
        app.save_circuit_to_yaml();
    }
    if is_ctrl_held && ctx.input(|i| i.key_pressed(egui::Key::O)) {
        if let Some(path) = open_file_dialog() {
            app.load_circuit_from_yaml(path);
        }
    }
    if is_ctrl_held && ctx.input(|i| i.key_pressed(egui::Key::N)) {
        app.new_circuit();
    }
    if is_ctrl_held && !modifiers.shift && ctx.input(|i| i.key_pressed(egui::Key::Z)) {
        app.undo_last_change();
    }
    if is_ctrl_held && ctx.input(|i| i.key_pressed(egui::Key::Y)) {
        app.redo_last_change();
    }
    if is_ctrl_held && modifiers.shift && ctx.input(|i| i.key_pressed(egui::Key::Z)) {
        app.redo_last_change();
    }
    if is_ctrl_held && ctx.input(|i| i.key_pressed(egui::Key::D)) {
        app.duplicate_selected_component();
    }

    // Delete key — only when no text field is focused
    if ctx.input(|i| i.key_pressed(egui::Key::Delete))
        && ctx.memory(|m| m.focused().is_none())
    {
        app.delete_selected_component();
    }
}

// ── Top menu bar ────────────────────────────────────────────────────────────

fn draw_top_menu_bar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New          Ctrl+N").clicked() {
                    app.new_circuit();
                    ui.close();
                }
                if ui.button("Open...      Ctrl+O").clicked() {
                    if let Some(path) = open_file_dialog() {
                        app.load_circuit_from_yaml(path);
                    }
                    ui.close();
                }
                if ui.button("Save         Ctrl+S").clicked() {
                    app.save_circuit_to_yaml();
                    ui.close();
                }
                if ui.button("Save As...").clicked() {
                    if let Some(path) = save_file_dialog() {
                        app.file_path = Some(path);
                        app.save_circuit_to_yaml();
                    }
                    ui.close();
                }
            });

            ui.menu_button("Edit", |ui| {
                let undo_label = if app.undo_stack.can_undo() {
                    "Undo         Ctrl+Z"
                } else {
                    "Undo         (nothing)"
                };
                if ui
                    .add_enabled(app.undo_stack.can_undo(), egui::Button::new(undo_label))
                    .clicked()
                {
                    app.undo_last_change();
                    ui.close();
                }

                let redo_label = if app.undo_stack.can_redo() {
                    "Redo         Ctrl+Y"
                } else {
                    "Redo         (nothing)"
                };
                if ui
                    .add_enabled(app.undo_stack.can_redo(), egui::Button::new(redo_label))
                    .clicked()
                {
                    app.redo_last_change();
                    ui.close();
                }

                ui.separator();

                if ui
                    .add_enabled(
                        app.selected_component.is_some(),
                        egui::Button::new("Duplicate    Ctrl+D"),
                    )
                    .clicked()
                {
                    app.duplicate_selected_component();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        app.selected_component.is_some(),
                        egui::Button::new("Delete       Del"),
                    )
                    .clicked()
                {
                    app.delete_selected_component();
                    ui.close();
                }
            });

            ui.separator();

            if ui.button("Simulate").clicked() {
                app.run_simulation();
            }

            if app.wire_state.source_component_index.is_some() {
                ui.separator();
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Wiring... click an input port (Esc to cancel)",
                );
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(&app.status_message);
            });
        });
    });
}

// ── Breadcrumb navigation bar ───────────────────────────────────────────

fn draw_breadcrumb_bar(app: &mut App, ctx: &egui::Context) {
    if app.editing_abstraction_index.is_none() {
        return;
    }

    egui::TopBottomPanel::top("breadcrumb").show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.link("Main Circuit").clicked() {
                app.editing_abstraction_index = None;
                app.selected_component = None;
                app.status_message = "Back to main circuit".into();
            }
            ui.label(">");
            if let Some(abs_idx) = app.editing_abstraction_index {
                if abs_idx < app.circuit.abstractions.len() {
                    ui.strong(&app.circuit.abstractions[abs_idx].name);
                }
            }
            ui.label("  (Esc or Backspace to go back)");
        });
    });
}

// ── Escape key handling ─────────────────────────────────────────────────────

fn handle_escape_cancel_wire(app: &mut App, ctx: &egui::Context) {
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        if app.wire_state.source_component_index.is_some() {
            app.wire_state.source_component_index = None;
            app.wire_state.source_output_name = None;
            app.status_message = "Wire cancelled".into();
        } else if app.editing_abstraction_index.is_some() {
            app.editing_abstraction_index = None;
            app.selected_component = None;
            app.status_message = "Back to main circuit".into();
        }
    }

    // Backspace also goes back to main circuit when no text field is focused
    if ctx.input(|i| i.key_pressed(egui::Key::Backspace))
        && ctx.memory(|m| m.focused().is_none())
        && app.editing_abstraction_index.is_some()
    {
        app.editing_abstraction_index = None;
        app.selected_component = None;
        app.status_message = "Back to main circuit".into();
    }
}

// ── Right side panel ────────────────────────────────────────────────────────

fn draw_right_side_panel(app: &mut App, ctx: &egui::Context) {
    egui::SidePanel::right("props")
        .min_width(300.0)
        .show(ctx, |ui| {
            // Tab bar
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut app.active_panel_tab,
                    PanelTab::Components,
                    "Components",
                );
                ui.selectable_value(
                    &mut app.active_panel_tab,
                    PanelTab::Abstractions,
                    "Abstractions",
                );
            });
            ui.separator();

            match app.active_panel_tab {
                PanelTab::Components => app.draw_components_panel(ui),
                PanelTab::Abstractions => app.draw_abstractions_panel(ui),
            }

            app.draw_simulation_results_table(ui);
        });
}
