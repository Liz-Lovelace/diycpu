//! The two modal dialogs (component layout, image settings): shared
//! geometry for the renderer and hit-testing, plus their mouse handling.
//! Both are pure preview state — edits commit on release/confirm.


use diycpu::editor::SheetRef;
use diycpu::layout::{self, PortKind, Rect};


use super::*;


impl App {
    pub fn action_open_layout_dialog(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        let SheetRef::Abstraction(index) = self.editor.sheet else {
            return;
        };
        self.gesture = Gesture::Idle;
        self.layout_dialog = Some(LayoutDialog {
            index,
            drag: None,
            pending_port: None,
            pending_size: None,
        });
    }

    /// The dialog's abstraction with pending preview edits applied.
    pub fn dialog_def(&self) -> Option<diycpu::project::ProjectAbstraction> {
        let dialog = self.layout_dialog.as_ref()?;
        let mut def = self
            .editor
            .project
            .abstractions
            .get(dialog.index)?
            .clone();
        if let Some((name, side, t)) = &dialog.pending_port {
            def.port_layout.insert(
                name.clone(),
                diycpu::project::PortSpot { side: *side, t: *t },
            );
        }
        if let Some((w, h)) = dialog.pending_size {
            def.width = w;
            def.height = h;
        }
        Some(def)
    }

    pub fn dialog_geometry(&self) -> Option<DialogGeom> {
        let def = self.dialog_def()?;
        let panel_w = 560.0f32;
        let panel_h = 460.0f32;
        let panel = Rect {
            x: (self.window_size[0] - panel_w) / 2.0,
            y: (self.window_size[1] - panel_h) / 2.5,
            w: panel_w,
            h: panel_h,
        };
        let area = Rect {
            x: panel.x + 70.0,
            y: panel.y + 80.0,
            w: panel.w - 140.0,
            h: panel.h - 170.0,
        };
        let scale = (area.w / def.width).min(area.h / def.height);
        let pw = def.width * scale;
        let ph = def.height * scale;
        let preview = Rect {
            x: area.x + (area.w - pw) / 2.0,
            y: area.y + (area.h - ph) / 2.0,
            w: pw,
            h: ph,
        };

        // Port positions through the same layout code the canvas uses.
        let mut abstractions = self.editor.project.abstractions.clone();
        abstractions[self.layout_dialog.as_ref()?.index] = def.clone();
        let synthetic = diycpu::project::ProjectComponent {
            id: String::new(),
            component_type: def.name.clone(),
            init: None,
            bits: None,
            pos: [0.0, 0.0],
        };
        let ports = layout::component_ports(&abstractions, &synthetic)
            .into_iter()
            .map(|p| {
                (
                    p.info.name.clone(),
                    [
                        preview.x + p.pos[0] * scale,
                        preview.y + p.pos[1] * scale,
                    ],
                    p.info.kind == PortKind::Input,
                )
            })
            .collect();

        let slider_row = |row: usize, label: &'static str, value: f32| DialogSlider {
            label,
            track: Rect {
                x: panel.x + 90.0,
                y: panel.y + panel.h - 72.0 + row as f32 * 30.0,
                w: panel.w - 220.0,
                h: 14.0,
            },
            value_rect: Rect {
                x: panel.x + panel.w - 110.0,
                y: panel.y + panel.h - 78.0 + row as f32 * 30.0,
                w: 84.0,
                h: 24.0,
            },
            value,
        };
        Some(DialogGeom {
            panel,
            sliders: [
                slider_row(0, "Width", def.width),
                slider_row(1, "Height", def.height),
            ],
            close_rect: Rect {
                x: panel.x + panel.w - 34.0,
                y: panel.y + 10.0,
                w: 24.0,
                h: 24.0,
            },
            preview,
            ports,
            size: (def.width, def.height),
        })
    }

    pub fn dialog_mouse_pressed(&mut self) {
        let Some(geom) = self.dialog_geometry() else {
            return;
        };
        if geom.close_rect.contains(self.cursor) || !geom.panel.contains(self.cursor) {
            self.layout_dialog = None;
            return;
        }
        for (i, slider) in geom.sliders.iter().enumerate() {
            // A generous grab zone around the track.
            let grab = Rect {
                x: slider.track.x - 8.0,
                y: slider.track.y - 8.0,
                w: slider.track.w + 16.0,
                h: slider.track.h + 16.0,
            };
            if grab.contains(self.cursor) {
                if let Some(d) = &mut self.layout_dialog {
                    d.drag = Some(DialogDrag::Slider(i));
                }
                self.dialog_mouse_moved(); // jump the value to the click
                return;
            }
            if slider.value_rect.contains(self.cursor) {
                // Type the number instead.
                let index = self.layout_dialog.as_ref().unwrap().index;
                self.prompt = Some(Prompt {
                    title: format!("{} in grid cells", slider.label),
                    value: format!("{}", slider.value),
                    action: PromptAction::SetAbstractionDim { index, which: i },
                    identifier: false,
                });
                return;
            }
        }
        for (name, pos, _) in &geom.ports {
            let dx = self.cursor[0] - pos[0];
            let dy = self.cursor[1] - pos[1];
            if (dx * dx + dy * dy).sqrt() <= 12.0 {
                if let Some(d) = &mut self.layout_dialog {
                    d.drag = Some(DialogDrag::Port(name.clone()));
                }
                return;
            }
        }
    }

    pub fn dialog_mouse_moved(&mut self) {
        let Some(geom) = self.dialog_geometry() else {
            return;
        };
        let cursor = self.cursor;
        let Some(dialog) = &mut self.layout_dialog else {
            return;
        };
        match dialog.drag.clone() {
            Some(DialogDrag::Port(name)) => {
                // Project the cursor onto the nearest edge of the preview.
                let r = geom.preview;
                let x = cursor[0].clamp(r.x, r.x + r.w);
                let y = cursor[1].clamp(r.y, r.y + r.h);
                let d_left = (cursor[0] - r.x).abs();
                let d_right = (cursor[0] - (r.x + r.w)).abs();
                let d_top = (cursor[1] - r.y).abs();
                let d_bottom = (cursor[1] - (r.y + r.h)).abs();
                let min = d_left.min(d_right).min(d_top).min(d_bottom);
                use diycpu::project::PortSide;
                let (side, t) = if min == d_left {
                    (PortSide::Left, (y - r.y) / r.h)
                } else if min == d_right {
                    (PortSide::Right, (y - r.y) / r.h)
                } else if min == d_top {
                    (PortSide::Top, (x - r.x) / r.w)
                } else {
                    (PortSide::Bottom, (x - r.x) / r.w)
                };
                // Snap to twentieths for tidy alignment.
                let t = ((t * 20.0).round() / 20.0).clamp(0.02, 0.98);
                dialog.pending_port = Some((name, side, t));
            }
            Some(DialogDrag::Slider(i)) => {
                let track = &geom.sliders[i].track;
                let f = ((cursor[0] - track.x) / track.w).clamp(0.0, 1.0);
                let value = (DIALOG_SIZE_MIN + f * (DIALOG_SIZE_MAX - DIALOG_SIZE_MIN))
                    .round();
                let (w, h) = geom.size;
                dialog.pending_size = Some(if i == 0 { (value, h) } else { (w, value) });
            }
            None => {}
        }
    }

    pub fn dialog_mouse_released(&mut self) {
        let Some(dialog) = &mut self.layout_dialog else {
            return;
        };
        let index = dialog.index;
        let pending_port = dialog.pending_port.take();
        let pending_size = dialog.pending_size.take();
        let had_drag = dialog.drag.take();
        match had_drag {
            Some(DialogDrag::Port(_)) => {
                if let Some((name, side, t)) = pending_port {
                    if let Err(e) = self.editor.set_port_layout(index, &name, side, t) {
                        self.error(e.to_string());
                    }
                }
            }
            Some(DialogDrag::Slider(_)) => {
                if let Some((w, h)) = pending_size {
                    if let Err(e) = self.editor.set_abstraction_size(index, w, h) {
                        self.error(e.to_string());
                    }
                }
            }
            None => {}
        }
    }

    pub fn action_open_image_dialog(&mut self, id: &str) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        self.image_dialog = Some(ImageDialog {
            id: id.to_string(),
            drag: None,
            pending_size: None,
        });
    }

    /// The dialog's image with pending preview edits applied.
    pub fn image_dialog_image(&self) -> Option<diycpu::project::MediaImage> {
        let dialog = self.image_dialog.as_ref()?;
        let mut img = self
            .editor
            .sheet_images()
            .iter()
            .find(|i| i.id == dialog.id)?
            .clone();
        if let Some((w, h)) = dialog.pending_size {
            img.size = [w, h];
        }
        Some(img)
    }

    pub fn image_dialog_geometry(&self) -> Option<ImageDialogGeom> {
        let img = self.image_dialog_image()?;
        let panel_w = 460.0f32;
        let panel_h = 440.0f32;
        let panel = Rect {
            x: (self.window_size[0] - panel_w) / 2.0,
            y: (self.window_size[1] - panel_h) / 2.5,
            w: panel_w,
            h: panel_h,
        };
        // Aspect-fit preview of the picture (by its display proportions).
        let area = Rect {
            x: panel.x + 30.0,
            y: panel.y + 56.0,
            w: panel.w - 60.0,
            h: panel.h - 240.0,
        };
        let (gw, gh) = (img.size[0].max(0.1), img.size[1].max(0.1));
        let scale = (area.w / gw).min(area.h / gh);
        let (pw, ph) = (gw * scale, gh * scale);
        let preview = Rect {
            x: area.x + (area.w - pw) / 2.0,
            y: area.y + (area.h - ph) / 2.0,
            w: pw,
            h: ph,
        };
        let choose_rect = Rect {
            x: panel.x + 30.0,
            y: panel.y + panel.h - 156.0,
            w: panel.w - 60.0,
            h: 30.0,
        };
        let slider_row = |row: usize, label: &'static str, value: f32| DialogSlider {
            label,
            track: Rect {
                x: panel.x + 90.0,
                y: panel.y + panel.h - 96.0 + row as f32 * 32.0,
                w: panel.w - 220.0,
                h: 14.0,
            },
            value_rect: Rect {
                x: panel.x + panel.w - 110.0,
                y: panel.y + panel.h - 102.0 + row as f32 * 32.0,
                w: 84.0,
                h: 24.0,
            },
            value,
        };
        Some(ImageDialogGeom {
            panel,
            preview,
            file_label: img.file.clone(),
            choose_rect,
            sliders: [
                slider_row(0, "Width", img.size[0]),
                slider_row(1, "Height", img.size[1]),
            ],
            close_rect: Rect {
                x: panel.x + panel_w - 34.0,
                y: panel.y + 10.0,
                w: 24.0,
                h: 24.0,
            },
        })
    }

    pub fn img_dialog_pressed(&mut self) {
        let Some(geom) = self.image_dialog_geometry() else {
            self.image_dialog = None;
            return;
        };
        if geom.close_rect.contains(self.cursor) || !geom.panel.contains(self.cursor) {
            self.image_dialog = None;
            return;
        }
        if geom.choose_rect.contains(self.cursor) {
            self.choose_image_for_dialog();
            return;
        }
        for (i, slider) in geom.sliders.iter().enumerate() {
            let grab = Rect {
                x: slider.track.x - 8.0,
                y: slider.track.y - 8.0,
                w: slider.track.w + 16.0,
                h: slider.track.h + 16.0,
            };
            if grab.contains(self.cursor) {
                if let Some(d) = &mut self.image_dialog {
                    d.drag = Some(i);
                }
                self.img_dialog_moved(); // jump the value to the click
                return;
            }
            if slider.value_rect.contains(self.cursor) {
                let id = self.image_dialog.as_ref().unwrap().id.clone();
                self.prompt = Some(Prompt {
                    title: format!("{} in grid cells", slider.label),
                    value: format!("{}", slider.value),
                    action: PromptAction::SetImageDim { id, which: i },
                    identifier: false,
                });
                return;
            }
        }
    }

    pub fn img_dialog_moved(&mut self) {
        let Some(geom) = self.image_dialog_geometry() else {
            return;
        };
        let cursor = self.cursor;
        let Some(dialog) = &mut self.image_dialog else {
            return;
        };
        if let Some(i) = dialog.drag {
            let track = &geom.sliders[i].track;
            let f = ((cursor[0] - track.x) / track.w).clamp(0.0, 1.0);
            // Half-cell steps: fine enough for pictures, still tidy.
            let value =
                ((IMG_SIZE_MIN + f * (IMG_SIZE_MAX - IMG_SIZE_MIN)) * 2.0).round() / 2.0;
            let current = (geom.sliders[0].value, geom.sliders[1].value);
            dialog.pending_size = Some(if i == 0 {
                (value, current.1)
            } else {
                (current.0, value)
            });
        }
    }

    pub fn img_dialog_released(&mut self) {
        let Some(dialog) = &mut self.image_dialog else {
            return;
        };
        let id = dialog.id.clone();
        let pending = dialog.pending_size.take();
        if dialog.drag.take().is_some() {
            if let Some((w, h)) = pending {
                if let Err(e) = self.editor.set_image_size(&id, w, h) {
                    self.error(e.to_string());
                }
            }
        }
    }

    pub(super) fn choose_image_for_dialog(&mut self) {
        let Some(dialog) = &self.image_dialog else {
            return;
        };
        let id = dialog.id.clone();
        let picked = rfd::FileDialog::new()
            .add_filter("images", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file();
        let Some(path) = picked else {
            self.info("Cancelled");
            return;
        };
        let dims = match Self::validate_image_file(&path) {
            Ok(d) => d,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        let rel = match self.import_media(&path) {
            Ok(r) => r,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        // Keep the current width; the new file brings its own aspect.
        let w = self
            .image_dialog_image()
            .map(|i| i.size[0])
            .unwrap_or(6.0);
        let h = ((w * dims.1 as f32 / dims.0.max(1) as f32) * 2.0).round() / 2.0;
        match self
            .editor
            .set_image_file(&id, &rel, Some([w, h.clamp(IMG_SIZE_MIN, IMG_SIZE_MAX)]))
        {
            Ok(()) => self.info(format!("Image set to {rel}")),
            Err(e) => self.error(e.to_string()),
        }
    }
}

/// The "component layout" dialog: arrange ports on the closed rectangle's
/// edges and resize it. Pure preview state; edits commit on drag release.
pub struct LayoutDialog {
    pub index: usize,
    pub drag: Option<DialogDrag>,
    pub pending_port: Option<(String, diycpu::project::PortSide, f32)>,
    pub pending_size: Option<(f32, f32)>,
}

#[derive(Clone, PartialEq)]
pub enum DialogDrag {
    Port(String),
    /// Dragging one of the size sliders (0 = width, 1 = height).
    Slider(usize),
}

/// Everything the dialog's renderer and hit-testing share.
pub struct DialogGeom {
    pub panel: Rect,
    /// The preview rectangle of the closed component, in pixels.
    pub preview: Rect,
    /// (name, px position, is_input) for every port.
    pub ports: Vec<(String, [f32; 2], bool)>,
    /// Width and height sliders: (label, track, value_rect, value).
    pub sliders: [DialogSlider; 2],
    /// Current (possibly pending) size in grid units.
    pub size: (f32, f32),
    pub close_rect: Rect,
}

pub struct DialogSlider {
    pub label: &'static str,
    pub track: Rect,
    pub value_rect: Rect,
    pub value: f32,
}

/// Size sliders cover this range of grid cells.
pub const DIALOG_SIZE_MIN: f32 = 2.0;

pub const DIALOG_SIZE_MAX: f32 = 24.0;

/// The image dialog: swap the file, resize with sliders. Pure preview
/// state; edits commit on release / confirm.
pub struct ImageDialog {
    pub id: String,
    /// Index of the slider being dragged (0 = width, 1 = height).
    pub drag: Option<usize>,
    pub pending_size: Option<(f32, f32)>,
}

/// Everything the image dialog's renderer and hit-testing share.
pub struct ImageDialogGeom {
    pub panel: Rect,
    /// Where the picture itself is previewed, aspect-fit, in pixels.
    pub preview: Rect,
    pub file_label: String,
    pub choose_rect: Rect,
    pub sliders: [DialogSlider; 2],
    pub close_rect: Rect,
}

/// Image size sliders cover this range of grid cells.
pub const IMG_SIZE_MIN: f32 = 1.0;

pub const IMG_SIZE_MAX: f32 = 40.0;
