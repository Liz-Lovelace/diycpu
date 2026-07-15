//! Commands: everything a toolbar button, palette row, prompt or
//! shortcut can do. Small, name-shaped entry points that gestures and
//! the shell both call.

use std::path::PathBuf;

use diycpu::editor::{EditError, Editor, SheetRef};
use diycpu::layout::{PortKind, Rect};

use crate::sim::SimSession;

use super::*;


impl App {
    pub(super) fn toggle_run(&mut self) {
        self.playing = false;
        match self.mode {
            Mode::Run(_) => {
                self.mode = Mode::Edit;
                self.info("Back to editing");
            }
            Mode::Edit => {
                // The whole project simulates, but the view stays exactly
                // where the user is — zoomed into an abstraction included.
                match SimSession::new(&self.editor.project) {
                    Ok(session) => {
                        self.gesture = Gesture::Idle;
                        self.mode = Mode::Run(session);
                        self.info("Running — ←/→ step the clock, Esc to stop");
                    }
                    Err(e) => self.error(e),
                }
            }
        }
    }

    pub fn palette_clicked(&mut self, type_name: &str) {
        if !self.require_edit_mode() {
            return;
        }
        if type_name == "image" {
            // Armed via the file picker instead of a ghost type.
            if matches!(self.gesture, Gesture::PlacingImage { .. }) {
                self.gesture = Gesture::Idle;
                return;
            }
            self.action_place_image();
            return;
        }
        // Re-arming with the same item disarms it (toggle feel).
        if let Gesture::Placing { type_name: current } = &self.gesture {
            if current == type_name {
                self.gesture = Gesture::Idle;
                return;
            }
        }
        self.gesture = Gesture::Placing {
            type_name: type_name.to_string(),
        };
        self.info(format!(
            "Placing {type_name} — click the canvas (Shift-click for several), Esc cancels"
        ));
    }

    pub fn action_run_toggle(&mut self) {
        self.toggle_run();
    }

    pub fn action_step_forward(&mut self) {
        self.playing = false;
        if let Mode::Run(session) = &mut self.mode {
            session.step_forward();
        }
    }

    pub fn action_step_back(&mut self) {
        self.playing = false;
        if let Mode::Run(session) = &mut self.mode {
            session.step_back();
        }
    }

    pub fn action_rewind(&mut self) {
        self.playing = false;
        if let Mode::Run(session) = &mut self.mode {
            session.rewind();
        }
    }

    /// Toggle autoplay; from edit mode this also starts the run.
    pub fn action_play_toggle(&mut self) {
        match self.mode {
            Mode::Run(_) => self.playing = !self.playing,
            Mode::Edit => {
                self.toggle_run();
                if self.is_running() {
                    self.playing = true;
                }
            }
        }
        if self.playing {
            self.info("Playing — P pauses, ←/→ step by hand");
        }
    }

    /// One timer tick while playing.
    pub fn play_tick(&mut self) {
        if !self.playing {
            return;
        }
        if let Mode::Run(session) = &mut self.mode {
            session.step_forward();
        }
    }

    pub fn undo(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        self.selection.clear();
        let before = self.sheet_identity(self.focus_sheet());
        match self.editor.undo() {
            Ok(()) => self.info("Undone"),
            Err(e) => self.info(e.to_string()),
        }
        if self.editor.sheet != self.focus_sheet()
            || self.sheet_identity(self.editor.sheet) != before
        {
            self.reset_view_to_editor_sheet();
        }
    }

    pub fn redo(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        self.selection.clear();
        let before = self.sheet_identity(self.focus_sheet());
        match self.editor.redo() {
            Ok(()) => self.info("Redone"),
            Err(e) => self.info(e.to_string()),
        }
        if self.editor.sheet != self.focus_sheet()
            || self.sheet_identity(self.editor.sheet) != before
        {
            self.reset_view_to_editor_sheet();
        }
    }

    pub fn copy_selection(&mut self) {
        let fragment = self.editor.fragment(
            &self.selection.components,
            &self.selection.notes,
            &self.selection.images,
        );
        if fragment.is_empty() {
            self.info("Nothing selected to copy");
            return;
        }
        let n = fragment.components.len() + fragment.notes.len() + fragment.images.len();
        self.clipboard = Some(fragment);
        self.info(format!("Copied {n} item(s)"));
    }

    pub fn cut_selection(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        let fragment = self.editor.fragment(
            &self.selection.components,
            &self.selection.notes,
            &self.selection.images,
        );
        if fragment.is_empty() {
            self.info("Nothing selected to cut");
            return;
        }
        let ids = self.selection.components.clone();
        let note_ids = self.selection.notes.clone();
        let image_ids = self.selection.images.clone();
        match self.editor.delete_items(&ids, &note_ids, &image_ids) {
            Ok(()) => {
                let n =
                    fragment.components.len() + fragment.notes.len() + fragment.images.len();
                self.clipboard = Some(fragment);
                self.selection.clear();
                self.info(format!("Cut {n} item(s)"));
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    pub fn paste_clipboard(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        let Some(fragment) = self.clipboard.clone() else {
            self.info("Clipboard is empty");
            return;
        };
        // Land centered on the cursor when it's over the canvas, otherwise
        // just beside the originals.
        let delta = if self.canvas_rect().contains(self.cursor) {
            let bounds = fragment
                .bounds(&self.editor.project.abstractions)
                .unwrap_or(Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
            let g = self.cursor_grid();
            [
                (g[0] - bounds.x - bounds.w / 2.0).round() as i32,
                (g[1] - bounds.y - bounds.h / 2.0).round() as i32,
            ]
        } else {
            [1, 1]
        };
        match self.editor.paste_fragment(&fragment, delta, true) {
            Ok((new_ids, new_notes, new_images)) => {
                self.selection.clear();
                self.selection.components = new_ids;
                self.selection.notes = new_notes;
                self.selection.images = new_images;
                self.gesture = Gesture::Idle;
                self.info("Pasted");
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    pub fn duplicate(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        let fragment = self.editor.fragment(
            &self.selection.components,
            &self.selection.notes,
            &self.selection.images,
        );
        if fragment.is_empty() {
            self.info("Nothing selected to duplicate");
            return;
        }
        match self.editor.paste_fragment(&fragment, [1, 1], true) {
            Ok((new_ids, new_notes, new_images)) => {
                self.info(format!(
                    "Duplicated {} item(s)",
                    new_ids.len() + new_notes.len() + new_images.len()
                ));
                self.selection.clear();
                self.selection.components = new_ids;
                self.selection.notes = new_notes;
                self.selection.images = new_images;
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    pub fn dismiss_prompt(&mut self) {
        if self.prompt.take().is_some() {
            self.info("Cancelled");
        }
    }

    pub(super) fn prompt_key(&mut self, key: KeyInput) {
        let Some(prompt) = &mut self.prompt else {
            return;
        };
        match key {
            KeyInput::Escape => {
                self.prompt = None;
                self.info("Cancelled");
            }
            KeyInput::Backspace => {
                prompt.value.pop();
            }
            KeyInput::Enter => self.commit_prompt(),
            KeyInput::Char(ch) if !self.ctrl => {
                for c in ch.chars() {
                    let ok = if prompt.identifier {
                        // ':' carries the bundle-width suffix in pin prompts.
                        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':'
                    } else {
                        !c.is_control()
                    };
                    if ok {
                        prompt.value.push(c);
                    }
                }
            }
            KeyInput::Space if !prompt.identifier => {
                prompt.value.push(' ');
            }
            _ => {}
        }
    }

    pub(super) fn commit_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        let name = prompt.value.trim().to_string();
        // Only notes may commit empty (that's how they're removed).
        if name.is_empty() && !matches!(prompt.action, PromptAction::EditNote { .. }) {
            self.error("A name is required");
            self.prompt = Some(prompt);
            return;
        }
        let retry = |app: &mut App, e: EditError, prompt: Prompt| {
            app.error(e.to_string());
            app.prompt = Some(prompt);
        };
        match prompt.action.clone() {
            PromptAction::NewAbstraction => match self.editor.create_abstraction(&name) {
                Ok(i) => {
                    let _ = self.editor.open_sheet(SheetRef::Abstraction(i));
                    self.view_root = SheetRef::Abstraction(i);
                    self.path.clear();
                    self.selection.clear();
                    self.pending_fit = true;
                    self.info(format!(
                        "Created \"{name}\" — add pins and parts, then Esc to go back"
                    ));
                }
                Err(e) => retry(self, e, prompt),
            },
            PromptAction::AddInputPin | PromptAction::AddOutputPin => {
                let kind = if prompt.action == PromptAction::AddInputPin {
                    PortKind::Input
                } else {
                    PortKind::Output
                };
                // "name:8" makes a bundle pin carrying 8 lines.
                let (pin_name, bundle) = match name.split_once(':') {
                    Some((n, w)) => match w.parse::<u32>() {
                        Ok(w) => (n.to_string(), Some(w)),
                        Err(_) => {
                            self.error("After the colon goes the number of lines, e.g. data:8");
                            self.prompt = Some(prompt);
                            return;
                        }
                    },
                    None => (name.clone(), None),
                };
                match self.editor.add_interface_pin(kind, &pin_name, bundle) {
                    Ok(()) => match bundle {
                        Some(w) => {
                            self.info(format!("Added bundle pin \"{pin_name}\" ({w} lines)"))
                        }
                        None => self.info(format!("Added pin \"{pin_name}\"")),
                    },
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::NewNote { pos } => {
                match self.editor.place_note([pos[0] as f32, pos[1] as f32], &name) {
                    Ok(id) => {
                        self.selection.clear();
                        self.selection.notes = vec![id];
                        self.info("Note placed — double-click it to edit");
                    }
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::EditNote { id } => match self.editor.set_note_text(&id, &name) {
                Ok(()) => {
                    if name.is_empty() {
                        self.selection.notes.retain(|n| n != &id);
                        self.info("Note removed");
                    }
                }
                Err(e) => retry(self, e, prompt),
            },
            PromptAction::RenamePin { kind, old } => {
                let (pin_name, bundle) = match name.split_once(':') {
                    Some((n, w)) => match w.parse::<u32>() {
                        Ok(w) => (n.to_string(), Some(Some(w))),
                        Err(_) => {
                            self.error("After the colon goes the number of lines, e.g. data:8");
                            self.prompt = Some(prompt);
                            return;
                        }
                    },
                    // A plain rename keeps the pin's bundling as it is.
                    None => (name.clone(), None),
                };
                let renamed = if pin_name == old {
                    Ok(())
                } else {
                    self.editor.rename_interface_pin(kind, &old, &pin_name)
                };
                match renamed {
                    Ok(()) => {
                        if let Some(width) = bundle {
                            match self.editor.set_pin_bundle_width(&pin_name, width) {
                                Ok(()) => self.info(match width {
                                    Some(w) => format!(
                                        "Pin \"{pin_name}\" now carries {w} lines — mismatched wires flash until both ends agree"
                                    ),
                                    None => format!(
                                        "Pin \"{pin_name}\" is a single wire again — its old bundle wiring was disconnected"
                                    ),
                                }),
                                Err(e) => self.error(e.to_string()),
                            }
                        } else {
                            self.info(format!("Renamed pin to \"{pin_name}\""));
                        }
                    }
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::RenameAbstraction { index } => {
                match self.editor.rename_abstraction(index, &name) {
                    Ok(()) => self.info(format!("Renamed to \"{name}\"")),
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::RenameProject => match self.editor.rename_project(&name) {
                Ok(()) => self.info(format!("Project renamed to \"{name}\"")),
                Err(e) => retry(self, e, prompt),
            },
            PromptAction::SetBundleWidth { id } => {
                let Ok(width) = name.parse::<u32>() else {
                    self.error("Enter a number of lines (2–32)");
                    self.prompt = Some(prompt);
                    return;
                };
                match self.editor.set_bundle_width(&id, width) {
                    Ok(()) => self.info(format!(
                        "{width} lines — a mismatched bundle wire flashes until the other end agrees"
                    )),
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::SetDisplayDim { id, which } => {
                let Ok(value) = name.parse::<f32>() else {
                    self.error("Enter a number of grid cells");
                    self.prompt = Some(prompt);
                    return;
                };
                let value = value.clamp(DISPLAY_SIZE_MIN, DISPLAY_SIZE_MAX);
                let (components, _) = self.editor.sheet_data();
                let Some(comp) = components.iter().find(|c| c.id == id) else {
                    return;
                };
                let mut cfg = comp.display.clone().unwrap_or_default();
                if matches!(comp.component_type.as_str(), "light" | "rgb") {
                    let mut size = layout::light_size(comp);
                    size[which.min(1)] = value;
                    cfg.size = Some(size);
                } else {
                    cfg.scale = Some(value);
                }
                match self.editor.configure_display(&id, cfg, None) {
                    Ok(()) => self.info("Resized"),
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::NewProject => {
                let create = || -> Result<PathBuf, String> {
                    let base = projects_base_dir().ok_or("No home folder found")?;
                    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
                    let dir = create_project_dir(&base, &name)?;
                    let mut project = Editor::empty().project;
                    project.name = name.clone();
                    let json = serde_json::to_string_pretty(&project)
                        .map_err(|e| e.to_string())?;
                    let path = dir.join("project.json");
                    std::fs::write(&path, json + "\n").map_err(|e| e.to_string())?;
                    Ok(path)
                };
                match create() {
                    Ok(path) => {
                        self.open_path(path);
                        self.info(format!("Created project \"{name}\""));
                    }
                    Err(e) => {
                        self.error(e);
                        self.prompt = Some(prompt);
                    }
                }
            }
            PromptAction::SaveCopyAs => match self.save_copy_into(&name) {
                Ok(path) => {
                    self.file_path = Some(path);
                    self.info(format!("Saved a copy as \"{name}\" — now editing it"));
                }
                Err(e) => {
                    self.error(e);
                    self.prompt = Some(prompt);
                }
            },
            PromptAction::SetImageDim { id, which } => {
                let Ok(value) = name.parse::<f32>() else {
                    self.error("Enter a number of grid cells");
                    self.prompt = Some(prompt);
                    return;
                };
                let value = value.clamp(IMG_SIZE_MIN, IMG_SIZE_MAX);
                let current = self
                    .editor
                    .sheet_images()
                    .iter()
                    .find(|i| i.id == id)
                    .map(|i| i.size)
                    .unwrap_or([6.0, 4.0]);
                let (w, h) = if which == 0 {
                    (value, current[1])
                } else {
                    (current[0], value)
                };
                match self.editor.set_image_size(&id, w, h) {
                    Ok(()) => self.info("Resized"),
                    Err(e) => retry(self, e, prompt),
                }
            }
            PromptAction::SetAbstractionDim { index, which } => {
                let Ok(value) = name.parse::<f32>() else {
                    self.error("Enter a number of grid cells");
                    self.prompt = Some(prompt);
                    return;
                };
                let value = value.clamp(DIALOG_SIZE_MIN, DIALOG_SIZE_MAX);
                let current = self
                    .editor
                    .project
                    .abstractions
                    .get(index)
                    .map(|a| (a.width, a.height))
                    .unwrap_or((4.0, 2.0));
                let (w, h) = if which == 0 {
                    (value, current.1)
                } else {
                    (current.0, value)
                };
                match self.editor.set_abstraction_size(index, w, h) {
                    Ok(()) => self.info("Resized"),
                    Err(e) => retry(self, e, prompt),
                }
            }
        }
    }

    pub fn action_new_abstraction(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        self.prompt = Some(Prompt {
            title: "Name the new abstraction".into(),
            value: String::new(),
            action: PromptAction::NewAbstraction,
            identifier: true,
        });
    }

    pub fn action_add_pin(&mut self, kind: PortKind) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        self.prompt = Some(Prompt {
            title: match kind {
                PortKind::Input => "Name the input pin (name:8 for a bundle)".into(),
                PortKind::Output => "Name the output pin (name:8 for a bundle)".into(),
            },
            value: String::new(),
            action: match kind {
                PortKind::Input => PromptAction::AddInputPin,
                PortKind::Output => PromptAction::AddOutputPin,
            },
            identifier: true,
        });
    }

    /// Rename whatever the title bar names: the open abstraction, or the
    /// project itself.
    pub fn action_rename_title(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        match self.editor.sheet {
            SheetRef::Abstraction(index) => {
                let Some(def) = self.editor.project.abstractions.get(index) else {
                    return;
                };
                self.prompt = Some(Prompt {
                    title: "Rename abstraction".into(),
                    value: def.name.clone(),
                    action: PromptAction::RenameAbstraction { index },
                    identifier: true,
                });
            }
            SheetRef::Top => {
                self.prompt = Some(Prompt {
                    title: "Rename project".into(),
                    value: self.editor.project.name.clone(),
                    action: PromptAction::RenameProject,
                    identifier: false,
                });
            }
        }
    }

    /// Delete the abstraction whose sheet is open (undoable; refused while
    /// instances of it exist).
    pub fn action_delete_abstraction(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        let SheetRef::Abstraction(index) = self.editor.sheet else {
            return;
        };
        match self.editor.delete_abstraction(index) {
            Ok(()) => {
                self.reset_view_to_editor_sheet();
                self.selection.clear();
                self.gesture = Gesture::Idle;
                self.info("Abstraction deleted (Ctrl+Z restores it)");
            }
            Err(e) => self.error(e.to_string()),
        }
    }
}


