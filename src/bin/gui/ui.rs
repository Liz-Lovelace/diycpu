//! Chrome layout: where the palette items and toolbar buttons are, computed
//! as pure geometry from the app state. The renderer draws these rects and
//! the shell hit-tests them, so they can never disagree.

use diycpu::layout::Rect;

use crate::app::{App, Mode};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    RunToggle,
    Rewind,
    StepBack,
    StepForward,
    Fit,
    Save,
    NewAbstraction,
    AddInputPin,
    AddOutputPin,
    BackToTop,
}

pub struct Button {
    pub id: ButtonId,
    pub rect: Rect,
    pub label: String,
    pub enabled: bool,
    pub active: bool,
}

pub struct PaletteItem {
    pub type_name: String,
    pub rect: Rect,
    pub enabled: bool,
    pub armed: bool,
}

pub struct UiLayout {
    pub palette_items: Vec<PaletteItem>,
    pub section_headers: Vec<(String, f32)>, // label, y
    pub buttons: Vec<Button>,
    /// "step k" readout position (run mode), None otherwise.
    pub step_label: Option<(String, f32, f32)>,
}

pub fn ui_layout(app: &App) -> UiLayout {
    let mut palette_items = Vec::new();
    let mut section_headers = Vec::new();

    let x = 10.0;
    let w = theme::PALETTE_WIDTH - 20.0;
    let mut y = theme::TOPBAR_HEIGHT + 10.0;

    section_headers.push(("PRIMITIVES".to_string(), y + 12.0));
    y += 24.0;

    let clk_placed = {
        let (components, _) = app.editor.sheet_data();
        components.iter().any(|c| c.component_type == "clk")
    };
    let edited_abstraction = match app.editor.sheet {
        diycpu::editor::SheetRef::Abstraction(i) => {
            app.editor.project.abstractions.get(i).map(|a| a.name.clone())
        }
        diycpu::editor::SheetRef::Top => None,
    };

    let armed_type = match &app.gesture {
        crate::app::Gesture::Placing { type_name } => Some(type_name.clone()),
        _ => None,
    };

    for item in app.palette_items() {
        let is_abstraction_start = !matches!(item.as_str(), "clk" | "nand" | "dff")
            && !palette_items
                .iter()
                .any(|p: &PaletteItem| !matches!(p.type_name.as_str(), "clk" | "nand" | "dff"));
        if is_abstraction_start {
            y += 10.0;
            section_headers.push(("ABSTRACTIONS".to_string(), y + 12.0));
            y += 24.0;
        }
        // Mirror the editor's placement rules so the palette telegraphs them:
        // one clock, never inside abstractions; no recursive nesting.
        let enabled = match (item.as_str(), &edited_abstraction) {
            ("clk", None) => !clk_placed,
            ("clk", Some(_)) => false,
            (_, Some(host)) => !diycpu::editor::abstraction_uses(
                &app.editor.project.abstractions,
                &item,
                host,
            ),
            _ => true,
        };
        palette_items.push(PaletteItem {
            armed: armed_type.as_deref() == Some(item.as_str()),
            type_name: item,
            rect: Rect { x, y, w, h: 32.0 },
            enabled,
        });
        y += 36.0;
    }

    // ── Topbar buttons ──
    let mut buttons = Vec::new();
    let bh = 26.0;
    let by = (theme::TOPBAR_HEIGHT - bh) / 2.0;
    let running = matches!(app.mode, Mode::Run(_));

    let mut bx = theme::PALETTE_WIDTH + 12.0;
    buttons.push(Button {
        id: ButtonId::RunToggle,
        rect: Rect { x: bx, y: by, w: 74.0, h: bh },
        label: if running { "■ Stop" } else { "▶ Run" }.into(),
        enabled: true,
        active: running,
    });
    bx += 86.0;

    let mut step_label = None;
    match &app.mode {
        Mode::Run(session) => {
            for (id, label, w) in [
                (ButtonId::Rewind, "⟲", 30.0),
                (ButtonId::StepBack, "◂", 30.0),
                (ButtonId::StepForward, "▸", 30.0),
            ] {
                buttons.push(Button {
                    id,
                    rect: Rect { x: bx, y: by, w, h: bh },
                    label: label.into(),
                    enabled: match id {
                        ButtonId::Rewind | ButtonId::StepBack => session.cursor > 0,
                        _ => true,
                    },
                    active: false,
                });
                bx += w + 6.0;
                if id == ButtonId::StepBack {
                    let text = format!("step {}", session.cursor);
                    step_label = Some((text, bx + 34.0, theme::TOPBAR_HEIGHT / 2.0));
                    bx += 74.0;
                }
            }
        }
        Mode::Edit => {
            let editing_abstraction =
                !matches!(app.editor.sheet, diycpu::editor::SheetRef::Top);
            let edit_buttons: &[(ButtonId, &str, f32)] = if editing_abstraction {
                &[
                    (ButtonId::BackToTop, "◂ Back", 70.0),
                    (ButtonId::AddInputPin, "+ input", 76.0),
                    (ButtonId::AddOutputPin, "+ output", 86.0),
                ]
            } else {
                &[(ButtonId::NewAbstraction, "+ abstraction", 110.0)]
            };
            for (id, label, w) in edit_buttons {
                buttons.push(Button {
                    id: *id,
                    rect: Rect { x: bx, y: by, w: *w, h: bh },
                    label: (*label).into(),
                    enabled: true,
                    active: false,
                });
                bx += w + 8.0;
            }
        }
    }

    // Right-aligned buttons.
    let mut rx = app.window_size[0] - 12.0;
    for (id, label, w) in [(ButtonId::Save, "Save", 64.0), (ButtonId::Fit, "Fit view", 76.0)] {
        rx -= w;
        buttons.push(Button {
            id,
            rect: Rect { x: rx, y: by, w, h: bh },
            label: label.into(),
            enabled: !matches!(id, ButtonId::Save) || !running,
            active: false,
        });
        rx -= 8.0;
    }

    UiLayout {
        palette_items,
        section_headers,
        buttons,
        step_label,
    }
}
