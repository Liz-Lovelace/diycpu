//! The hover cogwheel: one affordance that leads to whatever settings
//! the hovered thing has.


use diycpu::layout::{self, Hit, Rect};


use super::*;


impl App {
    /// The gear affordance for whatever is hovered, if it has settings:
    /// its screen rect and what clicking it does.
    pub fn cog(&self) -> Option<(Rect, CogAction)> {
        if self.is_running() || self.any_modal() || !matches!(self.gesture, Gesture::Idle) {
            return None;
        }
        // Hovering a component's port still counts as hovering the
        // component: the gear sits near the top-right port slot, and it
        // must not flicker away as the pointer crosses a port's hit ring.
        let hovered_component = match &self.hover {
            Hit::Component { component_id } | Hit::Port { component_id, .. } => {
                Some(component_id.clone())
            }
            _ => None,
        };
        let (target, action) = match (&self.hover, hovered_component) {
            (_, Some(component_id)) => {
                let (components, _) = self.editor.sheet_data();
                let comp = components.iter().find(|c| c.id == component_id)?;
                let rect = layout::footprint(&self.editor.project.abstractions, comp);
                let action = match comp.component_type.as_str() {
                    "dff" | "const" => CogAction::ToggleInit {
                        id: comp.id.clone(),
                    },
                    "bundle" | "unbundle" => CogAction::BundleWidth {
                        id: comp.id.clone(),
                    },
                    "nand" | "clk" => return None, // nothing to configure
                    _ => CogAction::Layout {
                        index: self
                            .editor
                            .project
                            .abstractions
                            .iter()
                            .position(|a| a.name == comp.component_type)?,
                    },
                };
                (rect, action)
            }
            (Hit::Image { image_id }, _) => {
                let img = self
                    .editor
                    .sheet_images()
                    .iter()
                    .find(|i| &i.id == image_id)?;
                (
                    layout::image_bounds(img),
                    CogAction::ImageSettings {
                        id: img.id.clone(),
                    },
                )
            }
            (Hit::Note { note_id }, _) => {
                let note = self
                    .editor
                    .sheet_notes()
                    .iter()
                    .find(|n| &n.id == note_id)?;
                (
                    layout::note_bounds(note),
                    CogAction::EditNote {
                        id: note.id.clone(),
                    },
                )
            }
            _ => return None,
        };
        // Tucked into the top-right corner, INSIDE the footprint so hovering
        // the gear still hovers the thing it belongs to.
        let tl = self.camera.grid_to_screen([target.x, target.y]);
        let w = target.w * self.camera.cell_px();
        let h = target.h * self.camera.cell_px();
        if w < COG_SIZE * 2.2 || h < COG_SIZE * 1.3 {
            return None; // too small on screen for a corner button
        }
        Some((
            Rect {
                x: tl[0] + w - COG_SIZE - 4.0,
                y: tl[1] + 4.0,
                w: COG_SIZE,
                h: COG_SIZE,
            },
            action,
        ))
    }

    pub(super) fn perform_cog(&mut self, action: CogAction) {
        match action {
            CogAction::Layout { index } => {
                self.gesture = Gesture::Idle;
                self.layout_dialog = Some(LayoutDialog {
                    index,
                    drag: None,
                    pending_port: None,
                    pending_size: None,
                });
            }
            CogAction::ImageSettings { id } => self.action_open_image_dialog(&id),
            CogAction::EditNote { id } => {
                let text = self
                    .editor
                    .sheet_notes()
                    .iter()
                    .find(|n| n.id == id)
                    .map(|n| n.text.clone())
                    .unwrap_or_default();
                self.prompt = Some(Prompt {
                    title: "Edit note (empty text removes it)".into(),
                    value: text,
                    action: PromptAction::EditNote { id },
                    identifier: false,
                });
            }
            CogAction::BundleWidth { id } => {
                let current = {
                    let (components, _) = self.editor.sheet_data();
                    components
                        .iter()
                        .find(|c| c.id == id)
                        .map(layout::bits_of)
                        .unwrap_or(8)
                };
                self.prompt = Some(Prompt {
                    title: "Number of lines in the bundle (2–32)".into(),
                    value: format!("{current}"),
                    action: PromptAction::SetBundleWidth { id },
                    identifier: false,
                });
            }
            CogAction::ToggleInit { id } => match self.editor.toggle_dff_init(&id) {
                Ok(v) => self.info(format!(
                    "Starts at {} now",
                    if v { "1" } else { "0" }
                )),
                Err(e) => self.error(e.to_string()),
            },
        }
    }
}

/// What clicking the gear on a hovered thing does.
#[derive(Clone, PartialEq, Debug)]
pub enum CogAction {
    /// Open the layout dialog for this abstraction type.
    Layout { index: usize },
    /// Open the image settings dialog.
    ImageSettings { id: String },
    /// Open the note text prompt.
    EditNote { id: String },
    /// Flip a dff/const start value (the face shows it immediately).
    ToggleInit { id: String },
    /// Prompt for a bundler/unbundler's lane count.
    BundleWidth { id: String },
}

/// The gear button's screen-space size.
pub(crate) const COG_SIZE: f32 = 16.0;
