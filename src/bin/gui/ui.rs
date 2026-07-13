//! Chrome layout: where the palette items and toolbar buttons are, computed
//! as pure geometry from the app state. The renderer draws these rects and
//! the shell hit-tests them, so they can never disagree.

use diycpu::layout::Rect;

use crate::app::{App, Mode};
use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonId {
    RunToggle,
    PlayPause,
    Rewind,
    StepBack,
    StepForward,
    Fit,
    Save,
    Open,
    NewAbstraction,
    AddInputPin,
    AddOutputPin,
    RenameAbstraction,
    LayoutDialog,
    DeleteAbstraction,
    BackToTop,
}

pub struct Button {
    pub id: ButtonId,
    pub rect: Rect,
    pub label: String,
    /// Blender-style inline hotkey hint, drawn dim next to the label.
    pub hotkey: Option<&'static str>,
    pub enabled: bool,
    pub active: bool,
}

pub struct PaletteItem {
    pub type_name: String,
    pub rect: Rect,
    pub enabled: bool,
    pub armed: bool,
    /// The "open this abstraction" affordance on abstraction rows.
    pub edit_rect: Option<Rect>,
}

pub struct UiLayout {
    pub palette_items: Vec<PaletteItem>,
    pub section_headers: Vec<(String, f32)>, // label, y
    pub buttons: Vec<Button>,
    /// "step k" readout position (run mode), None otherwise.
    pub step_label: Option<(String, f32, f32)>,
    /// Clickable title region (click renames the project / abstraction).
    pub title_rect: Rect,
}

pub fn ui_layout(app: &App) -> UiLayout {
    let mut palette_items = Vec::new();
    let mut section_headers = Vec::new();

    let x = 10.0;
    let w = theme::PALETTE_WIDTH - 20.0;
    let mut y = theme::TOPBAR_HEIGHT + 10.0 - app.palette_scroll;

    section_headers.push(("PRIMITIVES".to_string(), y + 12.0));
    y += 24.0;

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

    let is_builtin =
        |name: &str| matches!(name, "clk" | "nand" | "dff" | "const" | "note" | "image");
    for item in app.palette_items() {
        let is_abstraction_start = !is_builtin(&item)
            && !palette_items
                .iter()
                .any(|p: &PaletteItem| !is_builtin(&p.type_name));
        if is_abstraction_start {
            y += 10.0;
            section_headers.push(("ABSTRACTIONS".to_string(), y + 12.0));
            y += 24.0;
        }
        // Mirror the editor's placement rules so the palette telegraphs
        // them: no recursive nesting. (Clocks are a shared signal and may
        // be placed anywhere, any number of times.)
        let enabled = match (item.as_str(), &edited_abstraction) {
            (_, Some(host)) => !diycpu::editor::abstraction_uses(
                &app.editor.project.abstractions,
                &item,
                host,
            ),
            _ => true,
        };
        let edit_rect = (!is_builtin(&item)).then_some(Rect {
            x: x + w - 26.0,
            y: y + 6.0,
            w: 20.0,
            h: 20.0,
        });
        palette_items.push(PaletteItem {
            armed: armed_type.as_deref() == Some(item.as_str()),
            type_name: item,
            rect: Rect { x, y, w, h: 32.0 },
            enabled,
            edit_rect,
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
        rect: Rect { x: bx, y: by, w: 112.0, h: bh },
        label: if running { "■ Stop" } else { "▶ Run" }.into(),
        hotkey: Some("Space"),
        enabled: true,
        active: running,
    });
    bx += 124.0;

    let mut step_label = None;
    match &app.mode {
        Mode::Run(session) => {
            buttons.push(Button {
                id: ButtonId::PlayPause,
                rect: Rect { x: bx, y: by, w: 82.0, h: bh },
                label: if app.playing { "Pause" } else { "Play" }.into(),
                hotkey: Some("P"),
                enabled: true,
                active: app.playing,
            });
            bx += 94.0;

            for (id, label, hotkey, w) in [
                (ButtonId::Rewind, "⟲", "Home", 66.0),
                (ButtonId::StepBack, "◂", "←", 52.0),
                (ButtonId::StepForward, "▸", "→", 52.0),
            ] {
                buttons.push(Button {
                    id,
                    rect: Rect { x: bx, y: by, w, h: bh },
                    label: label.into(),
                    hotkey: Some(hotkey),
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
            let edit_buttons: &[(ButtonId, &str, Option<&'static str>, f32)] =
                if editing_abstraction {
                    &[
                        (ButtonId::BackToTop, "◂ Back", Some("Esc"), 96.0),
                        (ButtonId::AddInputPin, "+ input", None, 76.0),
                        (ButtonId::AddOutputPin, "+ output", None, 86.0),
                        (ButtonId::RenameAbstraction, "Rename", None, 76.0),
                        (ButtonId::LayoutDialog, "Layout", None, 72.0),
                        (ButtonId::DeleteAbstraction, "Delete", None, 72.0),
                    ]
                } else {
                    &[(ButtonId::NewAbstraction, "+ abstraction", None, 110.0)]
                };
            for (id, label, hotkey, w) in edit_buttons {
                buttons.push(Button {
                    id: *id,
                    rect: Rect { x: bx, y: by, w: *w, h: bh },
                    label: (*label).into(),
                    hotkey: *hotkey,
                    enabled: true,
                    active: false,
                });
                bx += w + 8.0;
            }
        }
    }

    // Right-aligned buttons.
    let mut rx = app.window_size[0] - 12.0;
    for (id, label, hotkey, w) in [
        (ButtonId::Save, "Save copy", Some("Ctrl+Shift+S"), 148.0),
        (ButtonId::Open, "Open", Some("Ctrl+O"), 100.0),
        (ButtonId::Fit, "Fit view", Some("F"), 98.0),
    ] {
        rx -= w;
        buttons.push(Button {
            id,
            rect: Rect { x: rx, y: by, w, h: bh },
            label: label.into(),
            hotkey,
            enabled: !matches!(id, ButtonId::Save | ButtonId::Open) || !running,
            active: false,
        });
        rx -= 8.0;
    }

    // The title doubles as the rename affordance; approximate its width
    // from the displayed breadcrumb length.
    let title_len: usize = app
        .breadcrumb()
        .iter()
        .map(|p| p.chars().count() + 3)
        .sum();
    let title_rect = Rect {
        x: 8.0,
        y: 6.0,
        w: (title_len as f32 * 8.0 + 20.0).min(theme::PALETTE_WIDTH - 10.0),
        h: theme::TOPBAR_HEIGHT - 12.0,
    };

    UiLayout {
        palette_items,
        section_headers,
        buttons,
        step_label,
        title_rect,
    }
}
