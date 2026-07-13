//! Application state and interaction logic. No drawing happens here; the
//! renderer reads this state. No document mutation happens here either —
//! gestures collect intent and commit through `Editor` operations exactly
//! once, so cancelling a gesture (Esc, right-click, focus loss) is always
//! just dropping it.

use std::path::PathBuf;

use diycpu::editor::{Editor, Fragment, SheetRef, WirePoint};
use diycpu::layout::{self, Hit, PortKind, Rect};

use crate::camera::Camera;
use crate::sim::SimSession;
use crate::theme;


mod actions;
mod cog;
mod dialogs;
mod files;
mod gestures;
mod view;

pub use dialogs::{
    DialogSlider, ImageDialog, LayoutDialog,
    DIALOG_SIZE_MAX, DIALOG_SIZE_MIN, IMG_SIZE_MAX, IMG_SIZE_MIN,
};
pub use files::{
    create_project_dir, default_project_path, load_project, projects_base_dir, MAX_MEDIA_BYTES,
};
// Part of the app's vocabulary; only the test modules touch them today.
#[allow(unused_imports)]
pub use cog::CogAction;
#[allow(unused_imports)]
pub use files::starter_project;
pub use gestures::WireTarget;
pub use view::{resolve_lane_display, PathSeg};

pub(crate) use files::DiscardIntent;
pub(crate) use view::DESCEND_AT;

#[cfg(test)]
mod bundle_gui_tests;
#[cfg(test)]
mod clipboard_tests;
#[cfg(test)]
mod crud_tests;
#[cfg(test)]
mod image_and_cog_tests;
#[cfg(test)]
mod project_dir_tests;
#[cfg(test)]
mod round9_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod wire_selection_tests;
#[cfg(test)]
mod zoom_tests;

// The size difference between variants is fine: there is exactly one Mode.
#[allow(clippy::large_enum_variant)]
pub enum Mode {
    Edit,
    Run(SimSession),
}

/// The one active gesture. Every field is ephemeral: dropping a gesture
/// never requires cleanup because gestures don't touch the document.
pub enum Gesture {
    Idle,
    Panning {
        last: [f32; 2],
    },
    /// Armed from the palette; a ghost follows the cursor.
    Placing {
        type_name: String,
    },
    /// Left button down on a component or note; not yet decided click vs drag.
    PressedComponent {
        id: String,
        press_screen: [f32; 2],
        /// Ctrl was held: a click toggles selection, a drag copies.
        ctrl: bool,
    },
    PressedNote {
        id: String,
        press_screen: [f32; 2],
        ctrl: bool,
    },
    PressedImage {
        id: String,
        press_screen: [f32; 2],
        ctrl: bool,
    },
    /// A picked image file follows the cursor; a click places it (the file
    /// is only copied into the project on commit).
    PlacingImage {
        source: PathBuf,
        size: [f32; 2],
    },
    DraggingComponents {
        ids: Vec<String>,
        note_ids: Vec<String>,
        image_ids: Vec<String>,
        press_grid: [f32; 2],
        delta: [i32; 2],
        /// Ctrl-drag: the originals stay put and copies land at the delta.
        copying: bool,
    },
    /// Left button down on empty canvas; not yet decided click vs band.
    PressedEmpty {
        press_screen: [f32; 2],
    },
    BandSelecting {
        start_grid: [f32; 2],
    },
    /// Wire drawing: every click commits to the document immediately (a
    /// node, a connection), so this gesture holds only the current anchor.
    DrawingWire {
        anchor: WirePoint,
    },
    /// Left button down on a wire node; not yet decided click vs drag.
    PressedJunction {
        net_id: String,
        junction_id: String,
        press_screen: [f32; 2],
    },
    DraggingJunction {
        net_id: String,
        junction_id: String,
        pos: [f32; 2],
    },
    /// Left button down on a wire segment; a click selects it, a drag
    /// tears a new node out of it and carries it along.
    PressedWire {
        net_id: String,
        segment_index: usize,
        press_screen: [f32; 2],
        ctrl: bool,
    },
    /// Left button down on a pin's name plate; not yet decided click vs drag.
    PressedPin {
        kind: PortKind,
        name: String,
        press_screen: [f32; 2],
    },
    DraggingPin {
        name: String,
        pos: [f32; 2],
    },
}

#[derive(Default)]
pub struct Selection {
    pub components: Vec<String>,
    pub notes: Vec<String>,
    pub images: Vec<String>,
    /// Individual wire segments, by (net id, segment index).
    pub segments: Vec<(String, usize)>,
    /// Derived output-pin wires, by pin name.
    pub pin_wires: Vec<String>,
    /// A selected interface pin (kind, name) inside an abstraction sheet.
    pub pin: Option<(PortKind, String)>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.components.clear();
        self.notes.clear();
        self.images.clear();
        self.segments.clear();
        self.pin_wires.clear();
        self.pin = None;
    }
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
            && self.notes.is_empty()
            && self.images.is_empty()
            && self.segments.is_empty()
            && self.pin_wires.is_empty()
            && self.pin.is_none()
    }
}

/// What a text prompt, once confirmed, will do.
#[derive(Clone, PartialEq, Eq)]
pub enum PromptAction {
    NewAbstraction,
    AddInputPin,
    AddOutputPin,
    NewNote { pos: [i32; 2] },
    EditNote { id: String },
    RenamePin { kind: PortKind, old: String },
    RenameAbstraction { index: usize },
    RenameProject,
    /// Type a size for the layout dialog (which: 0 = width, 1 = height).
    SetAbstractionDim { index: usize, which: usize },
    /// Type a size for the image dialog (which: 0 = width, 1 = height).
    SetImageDim { id: String, which: usize },
    /// Lane count for a bundler/unbundler.
    SetBundleWidth { id: String },
    /// Name for a fresh project directory in the projects folder.
    NewProject,
    /// Name for a copy of the current project in the projects folder.
    SaveCopyAs,
}

pub struct Prompt {
    pub title: String,
    pub value: String,
    pub action: PromptAction,
    /// Identifier prompts filter input to name-safe characters; free-text
    /// prompts (notes, project title) accept anything printable.
    pub identifier: bool,
}

/// Keyboard input, abstracted away from the windowing library so the app
/// logic stays unit-testable.
pub enum KeyInput<'a> {
    Escape,
    Delete,
    Backspace,
    Enter,
    ArrowLeft,
    ArrowRight,
    Home,
    Space,
    Char(&'a str),
}

pub struct Status {
    pub text: String,
    pub is_error: bool,
}

pub struct App {
    pub editor: Editor,
    pub camera: Camera,
    /// The sheet the whole view is rooted at (Top, or an abstraction
    /// opened from the sidebar).
    pub view_root: SheetRef,
    /// Zoom path from the view root down to the focused sheet.
    pub path: Vec<PathSeg>,
    pub mode: Mode,
    pub gesture: Gesture,
    pub selection: Selection,
    pub status: Status,
    pub file_path: Option<PathBuf>,

    pub cursor: [f32; 2],
    pub window_size: [f32; 2],
    pub ctrl: bool,
    pub shift: bool,
    pub hover: Hit,
    pub prompt: Option<Prompt>,
    pub layout_dialog: Option<LayoutDialog>,
    pub image_dialog: Option<ImageDialog>,
    pub clipboard: Option<Fragment>,
    /// Run mode advances the clock on a timer.
    pub playing: bool,
    /// Scroll offset of the palette, for projects with many abstractions.
    pub palette_scroll: f32,
    /// Wall-clock seconds since launch; drives animated images.
    pub anim_seconds: f32,
    /// Set when an action would discard unsaved changes; repeating the
    /// action confirms it. Any other click clears it.
    pending_discard: Option<DiscardIntent>,
    /// Fit the view to content at the next opportunity (needs window size).
    pub pending_fit: bool,
}

impl App {
    pub fn new(file_path: Option<PathBuf>) -> Self {
        let mut status_text = "Ready — drag from the palette, or open a file".to_string();
        let mut is_error = false;

        let editor = match &file_path {
            Some(path) => match load_project(path) {
                Ok((project, notes)) => {
                    status_text = if notes.is_empty() {
                        format!("Opened \"{}\"", project.name)
                    } else {
                        is_error = true;
                        format!(
                            "Opened \"{}\" — repaired: {}",
                            project.name,
                            notes.join("; ")
                        )
                    };
                    Editor::new(project)
                }
                Err(e) => {
                    status_text = e;
                    is_error = true;
                    Editor::empty()
                }
            },
            None => Editor::empty(),
        };

        App {
            editor,
            camera: Camera::new(),
            view_root: SheetRef::Top,
            path: Vec::new(),
            mode: Mode::Edit,
            gesture: Gesture::Idle,
            selection: Selection::default(),
            status: Status {
                text: status_text,
                is_error,
            },
            file_path,
            cursor: [0.0, 0.0],
            window_size: [1280.0, 720.0],
            ctrl: false,
            shift: false,
            hover: Hit::Empty,
            prompt: None,
            layout_dialog: None,
            image_dialog: None,
            clipboard: None,
            playing: false,
            palette_scroll: 0.0,
            anim_seconds: 0.0,
            pending_discard: None,
            pending_fit: true,
        }
    }

    /// The abstraction whose sheet is open, if any.
    pub fn interface(&self) -> Option<&diycpu::project::ProjectAbstraction> {
        match self.editor.sheet {
            SheetRef::Abstraction(i) => self.editor.project.abstractions.get(i),
            SheetRef::Top => None,
        }
    }

    pub(super) fn info(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            is_error: false,
        };
    }

    pub(super) fn error(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            is_error: true,
        };
    }

    pub fn is_running(&self) -> bool {
        matches!(self.mode, Mode::Run(_))
    }

    /// Any modal layer open? Prompts and dialogs freeze the canvas.
    pub fn any_modal(&self) -> bool {
        self.prompt.is_some() || self.layout_dialog.is_some() || self.image_dialog.is_some()
    }

    pub fn canvas_rect(&self) -> Rect {
        Rect {
            x: theme::PALETTE_WIDTH,
            y: theme::TOPBAR_HEIGHT,
            w: (self.window_size[0] - theme::PALETTE_WIDTH).max(1.0),
            h: (self.window_size[1] - theme::TOPBAR_HEIGHT - theme::STATUSBAR_HEIGHT).max(1.0),
        }
    }

    pub fn cursor_grid(&self) -> [f32; 2] {
        self.camera.screen_to_grid(self.cursor)
    }

    pub fn recompute_hover(&mut self) {
        let g = self.cursor_grid();
        let (components, wire_nets) = self.editor.sheet_data();
        self.hover = if self.canvas_rect().contains(self.cursor) {
            layout::hit_test(
                &self.editor.project.abstractions,
                components,
                wire_nets,
                diycpu::editor::sheet_notes(&self.editor.project, self.editor.sheet)
                    .unwrap_or(&[]),
                diycpu::editor::sheet_images(&self.editor.project, self.editor.sheet)
                    .unwrap_or(&[]),
                self.interface(),
                g,
            )
        } else {
            Hit::Empty
        };
    }

    /// The palette entries: primitives and the note tool, then abstractions.
    pub fn palette_items(&self) -> Vec<String> {
        let mut items = vec![
            "clk".to_string(),
            "nand".to_string(),
            "dff".to_string(),
            "const".to_string(),
            "bundle".to_string(),
            "unbundle".to_string(),
            "note".to_string(),
            "image".to_string(),
        ];
        items.extend(self.editor.project.abstractions.iter().map(|a| a.name.clone()));
        items
    }

    pub(super) fn require_edit_mode(&mut self) -> bool {
        if self.is_running() {
            self.error("Editing is locked while running — press Esc to stop");
            false
        } else {
            true
        }
    }
}
