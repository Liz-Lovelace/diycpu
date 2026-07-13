//! Application state and interaction logic. No drawing happens here; the
//! renderer reads this state. No document mutation happens here either —
//! gestures collect intent and commit through `Editor` operations exactly
//! once, so cancelling a gesture (Esc, right-click, focus loss) is always
//! just dropping it.

use std::path::PathBuf;

use diycpu::editor::{EditError, Editor, Fragment, SheetRef, WirePoint};
use diycpu::layout::{self, Hit, PortKind, Rect};
use diycpu::project::{PortRef, Project};

use crate::camera::Camera;
use crate::sim::SimSession;
use crate::theme;

/// One level of the zoom path: which instance on `parent` we're inside.
#[derive(Clone, PartialEq)]
pub struct PathSeg {
    pub parent: SheetRef,
    pub instance_id: String,
    pub child: SheetRef,
}

/// Focus descends into an instance when it covers the whole central region
/// of the viewport (canvas inset by this fraction of its smaller dimension
/// per side) AND is comfortably larger than the screen; it ascends when the
/// enclosing instance stops covering a slightly smaller region. The gap is
/// hysteresis.
const DESCEND_COVER_INSET: f32 = 0.20;
const ASCEND_COVER_INSET: f32 = 0.24;
const DESCEND_AT: f32 = 1.25;
const ASCEND_AT: f32 = 1.10;

/// Pixels the cursor must travel before a press becomes a drag.
const DRAG_THRESHOLD_PX: f32 = 5.0;

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
    DraggingComponents {
        ids: Vec<String>,
        note_ids: Vec<String>,
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
        self.segments.clear();
        self.pin_wires.clear();
        self.pin = None;
    }
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
            && self.notes.is_empty()
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
    pub clipboard: Option<Fragment>,
    /// Run mode advances the clock on a timer.
    pub playing: bool,
    /// Scroll offset of the palette, for projects with many abstractions.
    pub palette_scroll: f32,
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
                        format!("Opened {}", path.display())
                    } else {
                        is_error = true;
                        format!("Opened {} — repaired: {}", path.display(), notes.join("; "))
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
            clipboard: None,
            playing: false,
            palette_scroll: 0.0,
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

    // ── Infinite zoom: focus descend/ascend ──────────────────────────────

    pub fn focus_sheet(&self) -> SheetRef {
        self.path.last().map(|s| s.child).unwrap_or(self.view_root)
    }

    fn viewport_min(&self) -> f32 {
        let r = self.canvas_rect();
        r.w.min(r.h)
    }

    fn canvas_center(&self) -> [f32; 2] {
        let r = self.canvas_rect();
        [r.x + r.w / 2.0, r.y + r.h / 2.0]
    }

    /// The canvas rect inset by `frac` of its smaller dimension per side —
    /// "what the user is looking at".
    fn central_box(&self, frac: f32) -> Rect {
        let r = self.canvas_rect();
        let inset = r.w.min(r.h) * frac;
        Rect {
            x: r.x + inset,
            y: r.y + inset,
            w: (r.w - inset * 2.0).max(1.0),
            h: (r.h - inset * 2.0).max(1.0),
        }
    }

    /// The child→parent transform of one zoom-path segment, recomputed from
    /// the given document state.
    pub fn seg_map_in(
        &self,
        project: &Project,
        seg: &PathSeg,
    ) -> Option<diycpu::layout::SheetMap> {
        let abstractions = &project.abstractions;
        let (components, _) = diycpu::editor::sheet_of(project, seg.parent)?;
        let comp = components.iter().find(|c| c.id == seg.instance_id)?;
        let def = abstractions.iter().find(|a| a.name == comp.component_type)?;
        Some(diycpu::layout::instance_inner_map(
            abstractions,
            def,
            layout::footprint(abstractions, comp),
        ))
    }

    pub fn seg_map(&self, seg: &PathSeg) -> Option<diycpu::layout::SheetMap> {
        self.seg_map_in(&self.editor.project, seg)
    }

    /// Keep the focused sheet in step with the camera: zooming into an
    /// instance until it fills the screen hands focus (and the camera's
    /// coordinate system) to its sheet; zooming back out hands it back.
    /// The rebased camera describes the identical visual transform, so the
    /// hand-off is invisible.
    pub fn update_focus(&mut self) {
        if !matches!(self.gesture, Gesture::Idle | Gesture::Panning { .. }) {
            return;
        }
        for _ in 0..16 {
            if !self.try_descend() && !self.try_ascend() {
                break;
            }
        }
    }

    fn try_descend(&mut self) -> bool {
        let view = self.camera.view();
        let v = self.viewport_min();
        let abstractions = &self.editor.project.abstractions;
        let (components, _) = self.editor.sheet_data();

        let central = self.central_box(DESCEND_COVER_INSET);

        let mut target: Option<(String, usize, layout::Rect)> = None;
        for comp in components {
            let Some(def_index) = abstractions
                .iter()
                .position(|a| a.name == comp.component_type)
            else {
                continue;
            };
            let rect = layout::footprint(abstractions, comp);
            let tl = view.to_screen([rect.x, rect.y]);
            let (w, h) = (rect.w * view.scale, rect.h * view.scale);
            // "We're looking at it": the instance covers the whole central
            // region of the screen (components can't overlap, so at most
            // one instance can).
            let covers = tl[0] <= central.x
                && tl[1] <= central.y
                && tl[0] + w >= central.x + central.w
                && tl[1] + h >= central.y + central.h;
            if w.max(h) > DESCEND_AT * v && covers {
                target = Some((comp.id.clone(), def_index, rect));
                break;
            }
        }
        let Some((instance_id, def_index, rect)) = target else {
            return false;
        };

        let def = &self.editor.project.abstractions[def_index];
        let map = diycpu::layout::instance_inner_map(
            &self.editor.project.abstractions,
            def,
            rect,
        );
        self.camera.set_view(view.through(&map));
        self.path.push(PathSeg {
            parent: self.editor.sheet,
            instance_id,
            child: SheetRef::Abstraction(def_index),
        });
        let _ = self.editor.open_sheet(SheetRef::Abstraction(def_index));
        self.selection.clear();
        self.recompute_hover();
        true
    }

    fn try_ascend(&mut self) -> bool {
        let Some(seg) = self.path.last().cloned() else {
            return false;
        };
        let Some(map) = self.seg_map(&seg) else {
            // The enclosing instance vanished (undo, delete): snap out.
            self.reset_view_to_editor_sheet();
            return false;
        };
        let parent_view = self.camera.view().through(&map.inverse());
        let abstractions = &self.editor.project.abstractions;
        let (parent_components, _) =
            diycpu::editor::sheet_of(&self.editor.project, seg.parent).unwrap_or((&[], &[]));
        let Some(comp) = parent_components.iter().find(|c| c.id == seg.instance_id) else {
            self.reset_view_to_editor_sheet();
            return false;
        };
        let rect = layout::footprint(abstractions, comp);
        let tl = parent_view.to_screen([rect.x, rect.y]);
        let (w, h) = (rect.w * parent_view.scale, rect.h * parent_view.scale);
        let v = self.viewport_min();
        let central = self.central_box(ASCEND_COVER_INSET);
        let covers = tl[0] <= central.x
            && tl[1] <= central.y
            && tl[0] + w >= central.x + central.w
            && tl[1] + h >= central.y + central.h;
        if w.max(h) >= ASCEND_AT * v && covers {
            return false;
        }
        self.camera.set_view(parent_view);
        self.path.pop();
        let _ = self.editor.open_sheet(seg.parent);
        self.selection.clear();
        self.recompute_hover();
        true
    }

    /// One explicit step outward (Esc / Back button), seamlessly rebased.
    fn ascend_once(&mut self) -> bool {
        let Some(seg) = self.path.last().cloned() else {
            return false;
        };
        if let Some(map) = self.seg_map(&seg) {
            self.camera.set_view(self.camera.view().through(&map.inverse()));
        }
        self.path.pop();
        let _ = self.editor.open_sheet(seg.parent);
        self.selection.clear();
        self.gesture = Gesture::Idle;
        self.recompute_hover();
        true
    }

    /// The name of the abstraction a sheet ref points at, if any — used to
    /// notice when an index silently changes identity across undo/redo.
    fn sheet_identity(&self, sheet: SheetRef) -> Option<String> {
        match sheet {
            SheetRef::Top => None,
            SheetRef::Abstraction(i) => {
                self.editor.project.abstractions.get(i).map(|a| a.name.clone())
            }
        }
    }

    /// The editor's sheet changed underneath the view (undo across sheets,
    /// abstraction deleted): re-root the view there.
    fn reset_view_to_editor_sheet(&mut self) {
        self.view_root = self.editor.sheet;
        self.path.clear();
        self.pending_fit = true;
    }

    /// Open an abstraction as the root of the view (sidebar affordance).
    pub fn open_as_root(&mut self, index: usize) {
        if !self.require_edit_mode() {
            return;
        }
        match self.editor.open_sheet(SheetRef::Abstraction(index)) {
            Ok(()) => {
                self.view_root = SheetRef::Abstraction(index);
                self.path.clear();
                self.selection.clear();
                self.gesture = Gesture::Idle;
                self.pending_fit = true;
                self.info("Editing the abstraction — every instance updates; Esc goes back");
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    /// The project as it should be DRAWN right now: mid-drag gestures are
    /// applied to a temporary copy so components render fully and wires
    /// re-route live, while the real document stays untouched until the
    /// gesture commits.
    pub fn display_project(&self) -> std::borrow::Cow<'_, Project> {
        use std::borrow::Cow;
        let sheet = self.editor.sheet;
        match &self.gesture {
            Gesture::DraggingComponents {
                ids,
                note_ids,
                delta,
                copying,
                ..
            } if *delta != [0, 0] => {
                let mut p = self.editor.project.clone();
                let (components, notes) = match sheet {
                    SheetRef::Top => (&mut p.components, &mut p.notes),
                    SheetRef::Abstraction(i) => {
                        let a = &mut p.abstractions[i];
                        (&mut a.components, &mut a.notes)
                    }
                };
                if *copying {
                    // Preview phantoms at the target; the originals stay.
                    let phantoms: Vec<_> = components
                        .iter()
                        .filter(|c| ids.contains(&c.id))
                        .map(|c| {
                            let mut ghost = c.clone();
                            ghost.id = format!("__copy_{}", ghost.id);
                            ghost.pos[0] += delta[0] as f32;
                            ghost.pos[1] += delta[1] as f32;
                            ghost
                        })
                        .collect();
                    components.extend(phantoms);
                    let note_phantoms: Vec<_> = notes
                        .iter()
                        .filter(|n| note_ids.contains(&n.id))
                        .map(|n| {
                            let mut ghost = n.clone();
                            ghost.id = format!("__copy_{}", ghost.id);
                            ghost.pos[0] += delta[0] as f32;
                            ghost.pos[1] += delta[1] as f32;
                            ghost
                        })
                        .collect();
                    notes.extend(note_phantoms);
                } else {
                    for c in components.iter_mut().filter(|c| ids.contains(&c.id)) {
                        c.pos[0] += delta[0] as f32;
                        c.pos[1] += delta[1] as f32;
                    }
                    for n in notes.iter_mut().filter(|n| note_ids.contains(&n.id)) {
                        n.pos[0] += delta[0] as f32;
                        n.pos[1] += delta[1] as f32;
                    }
                }
                Cow::Owned(p)
            }
            Gesture::DraggingJunction {
                net_id,
                junction_id,
                pos,
            } => {
                let mut p = self.editor.project.clone();
                let wire_nets = match sheet {
                    SheetRef::Top => &mut p.wire_nets,
                    SheetRef::Abstraction(i) => &mut p.abstractions[i].wire_nets,
                };
                if let Some(j) = wire_nets
                    .iter_mut()
                    .find(|n| &n.id == net_id)
                    .and_then(|n| n.junctions.iter_mut().find(|j| &j.id == junction_id))
                {
                    j.pos = *pos;
                }
                Cow::Owned(p)
            }
            Gesture::DraggingPin { name, pos } => {
                let mut p = self.editor.project.clone();
                if let SheetRef::Abstraction(i) = sheet {
                    p.abstractions[i].pin_positions.insert(name.clone(), *pos);
                }
                Cow::Owned(p)
            }
            _ => Cow::Borrowed(&self.editor.project),
        }
    }

    /// The title-bar trail: project ▸ root abstraction ▸ … ▸ focus.
    pub fn breadcrumb(&self) -> Vec<String> {
        let abstractions = &self.editor.project.abstractions;
        let name_of = |sheet: &SheetRef| match sheet {
            SheetRef::Top => None,
            SheetRef::Abstraction(i) => abstractions.get(*i).map(|a| a.name.clone()),
        };
        let mut parts = vec![self.editor.project.name.clone()];
        parts.extend(name_of(&self.view_root));
        for seg in &self.path {
            parts.extend(name_of(&seg.child));
        }
        parts
    }

    // ── Small helpers ────────────────────────────────────────────────────

    fn info(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            is_error: false,
        };
    }

    fn error(&mut self, text: impl Into<String>) {
        self.status = Status {
            text: text.into(),
            is_error: true,
        };
    }

    pub fn is_running(&self) -> bool {
        matches!(self.mode, Mode::Run(_))
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
            "note".to_string(),
        ];
        items.extend(self.editor.project.abstractions.iter().map(|a| a.name.clone()));
        items
    }

    /// Nominal footprint of a palette item's ghost (notes size to their text
    /// once typed; the ghost just shows where it lands).
    pub fn placement_size(&self, type_name: &str) -> [f32; 2] {
        if type_name == "note" {
            return [4.0, 1.2];
        }
        layout::type_size(&self.editor.project.abstractions, type_name).unwrap_or([3.0, 2.0])
    }

    /// Snapped top-left placement position so the ghost centers on the cursor.
    pub fn placement_pos(&self, type_name: &str) -> [i32; 2] {
        let size = self.placement_size(type_name);
        let g = self.cursor_grid();
        [
            (g[0] - size[0] / 2.0).round() as i32,
            (g[1] - size[1] / 2.0).round() as i32,
        ]
    }

    /// Would placing/moving these footprints collide? Used by ghost previews;
    /// the editor re-validates authoritatively on commit.
    pub fn placement_is_valid(&self, type_name: &str) -> bool {
        if type_name == "note" {
            return true; // annotations may sit anywhere
        }
        let abstractions = &self.editor.project.abstractions;
        let Some(size) = layout::type_size(abstractions, type_name) else {
            return false;
        };
        let pos = self.placement_pos(type_name);
        let rect = Rect {
            x: pos[0] as f32,
            y: pos[1] as f32,
            w: size[0],
            h: size[1],
        };
        let (components, _) = self.editor.sheet_data();
        components
            .iter()
            .all(|c| !layout::footprint(abstractions, c).intersects(&rect))
    }

    pub fn drag_is_valid(&self, ids: &[String], delta: [i32; 2], copying: bool) -> bool {
        let abstractions = &self.editor.project.abstractions;
        let (components, _) = self.editor.sheet_data();
        let moved: Vec<Rect> = components
            .iter()
            .filter(|c| ids.contains(&c.id))
            .map(|c| {
                let mut r = layout::footprint(abstractions, c);
                r.x += delta[0] as f32;
                r.y += delta[1] as f32;
                r
            })
            .collect();
        components
            .iter()
            // Copies must clear everything, the originals included.
            .filter(|c| copying || !ids.contains(&c.id))
            .all(|c| {
                let r = layout::footprint(abstractions, c);
                !moved.iter().any(|m| m.intersects(&r))
            })
    }

    /// Wire-direction kind of a port on the current sheet (pseudo-pins
    /// included), for preview coloring only — the editor is authoritative.
    fn port_wire_kind(&self, port: &PortRef) -> Option<PortKind> {
        if port.component_id.starts_with(diycpu::convert::INPUT_PREFIX) {
            return Some(PortKind::Output);
        }
        if port.component_id.starts_with(layout::OUTPUT_PREFIX) {
            return Some(PortKind::Input);
        }
        let (components, _) = self.editor.sheet_data();
        let comp = components.iter().find(|c| c.id == port.component_id)?;
        layout::type_ports(&self.editor.project.abstractions, &comp.component_type)?
            .into_iter()
            .find(|p| p.name == port.port_name)
            .map(|p| p.kind)
    }

    /// Does the wire being drawn already carry a driving source?
    fn anchor_drives(&self, anchor: &WirePoint) -> bool {
        match anchor {
            WirePoint::Port(p) => self.port_wire_kind(p) == Some(PortKind::Output),
            WirePoint::Junction { net_id, .. } => {
                let (_, wire_nets) = self.editor.sheet_data();
                wire_nets
                    .iter()
                    .find(|n| &n.id == net_id)
                    .is_some_and(|n| n.source.is_some())
            }
            WirePoint::New(_) => false,
        }
    }

    /// Where the wire being drawn currently ends: a hovered port or node
    /// (snapped), or the half-grid-snapped cursor.
    pub fn wire_target(&self) -> WireTarget {
        let Gesture::DrawingWire { anchor } = &self.gesture else {
            return WireTarget::Free(snap_half(self.cursor_grid()));
        };
        match &self.hover {
            Hit::Port {
                component_id,
                port_name,
                ..
            } => {
                let target = PortRef {
                    component_id: component_id.clone(),
                    port_name: port_name.clone(),
                };
                if matches!(anchor, WirePoint::Port(p) if *p == target) {
                    return WireTarget::Free(snap_half(self.cursor_grid()));
                }
                // Two drivers can't share a wire.
                if self.anchor_drives(anchor)
                    && self.port_wire_kind(&target) == Some(PortKind::Output)
                {
                    return WireTarget::Incompatible;
                }
                WireTarget::Port(target)
            }
            Hit::Junction {
                net_id,
                junction_id,
            } => {
                if matches!(anchor, WirePoint::Junction { junction_id: a, .. } if a == junction_id)
                {
                    return WireTarget::Free(snap_half(self.cursor_grid()));
                }
                // Merging two driven nets is invalid (unless same source).
                let (_, wire_nets) = self.editor.sheet_data();
                let target_source = wire_nets
                    .iter()
                    .find(|n| &n.id == net_id)
                    .and_then(|n| n.source.clone());
                let anchor_source = match anchor {
                    WirePoint::Port(p)
                        if self.port_wire_kind(p) == Some(PortKind::Output) =>
                    {
                        Some(p.clone())
                    }
                    WirePoint::Junction { net_id: a_net, .. } => wire_nets
                        .iter()
                        .find(|n| &n.id == a_net)
                        .and_then(|n| n.source.clone()),
                    _ => None,
                };
                match (anchor_source, target_source) {
                    (Some(a), Some(b)) if a != b => WireTarget::Incompatible,
                    _ => WireTarget::Node {
                        net_id: net_id.clone(),
                        junction_id: junction_id.clone(),
                    },
                }
            }
            _ => WireTarget::Free(snap_half(self.cursor_grid())),
        }
    }

    // ── File operations ──────────────────────────────────────────────────

    /// Save to the current file, or fall through to Save As when the
    /// project has never been saved.
    pub fn save(&mut self) {
        match self.file_path.clone() {
            Some(path) => self.write_to(path),
            None => self.save_as(),
        }
    }

    pub fn save_as(&mut self) {
        let default_name: String = self
            .editor
            .project
            .name
            .chars()
            .map(|c| if c == '/' || c == '\\' { '-' } else { c })
            .collect();
        let picked = rfd::FileDialog::new()
            .add_filter("circuit", &["json"])
            .set_file_name(format!("{default_name}.json"))
            .save_file();
        match picked {
            Some(path) => self.write_to(path),
            None => self.info("Save cancelled"),
        }
    }

    /// Write-through persistence: every committed change lands on disk as
    /// soon as the project has a file. Called by the shell after events.
    pub fn autosave(&mut self) {
        if !self.editor.dirty {
            return;
        }
        let Some(path) = self.file_path.clone() else {
            return; // nowhere to save yet — Save As picks the home
        };
        match self.persist(&path) {
            Ok(()) => {}
            Err(e) => self.error(format!("Autosave failed: {e}")),
        }
    }

    fn persist(&mut self, path: &std::path::Path) -> Result<(), String> {
        serde_json::to_string_pretty(&self.editor.project)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(path, json + "\n").map_err(|e| e.to_string()))
            .map(|()| self.editor.dirty = false)
    }

    fn write_to(&mut self, path: PathBuf) {
        match self.persist(&path) {
            Ok(()) => {
                self.file_path = Some(path.clone());
                self.info(format!("Saved {}", path.display()));
            }
            Err(e) => self.error(format!("Save failed: {e}")),
        }
    }

    /// True when it's safe to discard the document (clean, or confirmed).
    fn confirm_discard(&mut self, intent: DiscardIntent, again: &str) -> bool {
        if !self.editor.dirty || self.pending_discard == Some(intent) {
            self.pending_discard = None;
            return true;
        }
        self.pending_discard = Some(intent);
        self.error(format!("Unsaved changes — {again} again to discard them"));
        false
    }

    fn new_project(&mut self) {
        if !self.confirm_discard(DiscardIntent::New, "press Ctrl+N") {
            return;
        }
        self.editor = Editor::empty();
        self.file_path = None;
        self.view_root = SheetRef::Top;
        self.path.clear();
        self.selection.clear();
        self.gesture = Gesture::Idle;
        self.mode = Mode::Edit;
        self.pending_fit = true;
        self.info("New circuit");
    }

    /// The window wants to close. True = allow it. A dirty never-saved
    /// project asks for one more click (everything else autosaves).
    pub fn request_close(&mut self) -> bool {
        if self.file_path.is_some() {
            self.autosave();
            return true;
        }
        self.confirm_discard(DiscardIntent::Close, "close")
    }

    pub fn open(&mut self) {
        if !self.confirm_discard(DiscardIntent::Open, "choose Open") {
            return;
        }
        let picked = rfd::FileDialog::new()
            .add_filter("circuit", &["json"])
            .pick_file();
        let Some(path) = picked else {
            self.info("Open cancelled");
            return;
        };
        match load_project(&path) {
            Ok((project, notes)) => {
                self.editor = Editor::new(project);
                self.file_path = Some(path.clone());
                self.view_root = SheetRef::Top;
                self.path.clear();
                self.selection.clear();
                self.gesture = Gesture::Idle;
                self.mode = Mode::Edit;
                self.playing = false;
                self.pending_fit = true;
                if notes.is_empty() {
                    self.info(format!("Opened {}", path.display()));
                } else {
                    self.error(format!(
                        "Opened {} — repaired: {}",
                        path.display(),
                        notes.join("; ")
                    ));
                }
            }
            Err(e) => self.error(e),
        }
    }

    // ── Run mode ─────────────────────────────────────────────────────────

    fn toggle_run(&mut self) {
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

    fn require_edit_mode(&mut self) -> bool {
        if self.is_running() {
            self.error("Editing is locked while running — press Esc to stop");
            false
        } else {
            true
        }
    }

    // ── Content bounds & view fitting ────────────────────────────────────

    pub fn content_bounds(&self) -> Option<Rect> {
        let abstractions = &self.editor.project.abstractions;
        let (components, _) = self.editor.sheet_data();
        let mut bounds = layout::components_bounds(abstractions, components);
        // Interface pins float beside the content — include them.
        if let Some(def) = self.interface() {
            for pin in layout::interface_pins(abstractions, def) {
                bounds = Some(bounds.map_or(pin.plate, |b| b.union(&pin.plate)));
            }
        }
        bounds
    }

    pub fn fit_view(&mut self) {
        let content = self.content_bounds().unwrap_or(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        self.camera.fit(content, self.canvas_rect(), 60.0);
        self.recompute_hover();
    }

    // ── Mouse input ──────────────────────────────────────────────────────

    pub fn mouse_moved(&mut self, pos: [f32; 2]) {
        self.cursor = pos;
        // Modal layers freeze the canvas beneath them.
        if self.prompt.is_some() || self.layout_dialog.is_some() {
            return;
        }

        match &mut self.gesture {
            Gesture::Panning { last } => {
                let dx = pos[0] - last[0];
                let dy = pos[1] - last[1];
                *last = pos;
                self.camera.pan(dx, dy);
                self.update_focus();
            }
            Gesture::PressedComponent {
                id,
                press_screen,
                ctrl,
            } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    // Promote to drag: the pressed component leads; if it's
                    // part of the selection the whole selection comes along.
                    let copying = *ctrl;
                    let (ids, note_ids) = if self.selection.components.contains(id) {
                        (
                            self.selection.components.clone(),
                            self.selection.notes.clone(),
                        )
                    } else {
                        let ids = vec![id.clone()];
                        self.selection.clear();
                        self.selection.components = ids.clone();
                        (ids, vec![])
                    };
                    let press_grid = self.camera.screen_to_grid(*press_screen);
                    self.gesture = Gesture::DraggingComponents {
                        ids,
                        note_ids,
                        press_grid,
                        delta: [0, 0],
                        copying,
                    };
                }
            }
            Gesture::PressedNote {
                id,
                press_screen,
                ctrl,
            } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    let copying = *ctrl;
                    let (ids, note_ids) = if self.selection.notes.contains(id) {
                        (
                            self.selection.components.clone(),
                            self.selection.notes.clone(),
                        )
                    } else {
                        let note_ids = vec![id.clone()];
                        self.selection.clear();
                        self.selection.notes = note_ids.clone();
                        (vec![], note_ids)
                    };
                    let press_grid = self.camera.screen_to_grid(*press_screen);
                    self.gesture = Gesture::DraggingComponents {
                        ids,
                        note_ids,
                        press_grid,
                        delta: [0, 0],
                        copying,
                    };
                }
            }
            Gesture::DraggingComponents {
                press_grid, delta, ..
            } => {
                let g = self.camera.screen_to_grid(pos);
                *delta = [
                    (g[0] - press_grid[0]).round() as i32,
                    (g[1] - press_grid[1]).round() as i32,
                ];
            }
            Gesture::PressedEmpty { press_screen } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    self.gesture = Gesture::BandSelecting {
                        start_grid: self.camera.screen_to_grid(*press_screen),
                    };
                }
            }
            Gesture::PressedJunction {
                net_id,
                junction_id,
                press_screen,
            } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    self.gesture = Gesture::DraggingJunction {
                        net_id: net_id.clone(),
                        junction_id: junction_id.clone(),
                        pos: snap_half(self.camera.screen_to_grid(pos)),
                    };
                }
            }
            Gesture::DraggingJunction { pos: drag_pos, .. } => {
                *drag_pos = snap_half(self.camera.screen_to_grid(pos));
            }
            Gesture::PressedWire {
                net_id,
                segment_index,
                press_screen,
                ..
            } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    let net_id = net_id.clone();
                    let segment_index = *segment_index;
                    let grab = snap_half(self.camera.screen_to_grid(*press_screen));
                    match self.editor.insert_junction(&net_id, segment_index, grab) {
                        Ok(junction_id) => {
                            self.gesture = Gesture::DraggingJunction {
                                net_id,
                                junction_id,
                                pos: snap_half(self.camera.screen_to_grid(pos)),
                            };
                        }
                        Err(e) => {
                            self.error(e.to_string());
                            self.gesture = Gesture::Idle;
                        }
                    }
                }
            }
            Gesture::PressedPin {
                name, press_screen, ..
            } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    self.gesture = Gesture::DraggingPin {
                        name: name.clone(),
                        pos: snap_half(self.camera.screen_to_grid(pos)),
                    };
                }
            }
            Gesture::DraggingPin { pos: drag_pos, .. } => {
                *drag_pos = snap_half(self.camera.screen_to_grid(pos));
            }
            _ => {}
        }

        self.recompute_hover();
    }

    pub fn left_pressed(&mut self) {
        self.pending_discard = None;

        // A click outside an open prompt dismisses it (backstop; the shell
        // normally swallows these first).
        if self.prompt.is_some() {
            self.dismiss_prompt();
            return;
        }

        // UI chrome clicks are resolved by the shell (main.rs) before this
        // is called; here we only handle the canvas.
        if !self.canvas_rect().contains(self.cursor) {
            return;
        }

        match &self.gesture {
            Gesture::Placing { type_name } => {
                let type_name = type_name.clone();
                let pos = self.placement_pos(&type_name);
                if type_name == "note" {
                    // The note is only created once its text is confirmed.
                    self.gesture = Gesture::Idle;
                    self.prompt = Some(Prompt {
                        title: "Note text".into(),
                        value: String::new(),
                        action: PromptAction::NewNote { pos },
                        identifier: false,
                    });
                    return;
                }
                match self.editor.place_component(&type_name, pos) {
                    Ok(id) => {
                        self.selection.clear();
                        self.selection.components = vec![id];
                        self.info(format!("Placed a {type_name}"));
                        if !self.shift {
                            self.gesture = Gesture::Idle;
                        }
                    }
                    Err(e) => self.error(e.to_string()),
                }
                return;
            }
            Gesture::DrawingWire { .. } => {
                self.wire_click();
                return;
            }
            _ => {}
        }

        if self.is_running() {
            // Selection-only interactions while running.
            match self.hover.clone() {
                Hit::Component { component_id } => {
                    self.selection.clear();
                    self.selection.components = vec![component_id];
                }
                Hit::Wire {
                    net_id,
                    segment_index,
                } => {
                    self.selection.clear();
                    self.selection.segments = vec![(net_id, segment_index)];
                }
                _ => self.selection.clear(),
            }
            return;
        }

        match self.hover.clone() {
            Hit::Port {
                component_id,
                port_name,
                ..
            } => {
                self.gesture = Gesture::DrawingWire {
                    anchor: WirePoint::Port(PortRef {
                        component_id,
                        port_name,
                    }),
                };
                self.info(
                    "Wiring — click a port to connect, empty space to drop a node, right-click to stop",
                );
            }
            Hit::Junction {
                net_id,
                junction_id,
            } => {
                self.gesture = Gesture::PressedJunction {
                    net_id,
                    junction_id,
                    press_screen: self.cursor,
                };
            }
            Hit::Component { component_id } => {
                // Plain press selects for dragging; a ctrl press stays
                // undecided (click = toggle selection, drag = copy).
                if !self.ctrl && !self.selection.components.contains(&component_id) {
                    self.selection.clear();
                    self.selection.components = vec![component_id.clone()];
                }
                self.gesture = Gesture::PressedComponent {
                    id: component_id,
                    press_screen: self.cursor,
                    ctrl: self.ctrl,
                };
            }
            Hit::Note { note_id } => {
                if !self.ctrl && !self.selection.notes.contains(&note_id) {
                    self.selection.clear();
                    self.selection.notes = vec![note_id.clone()];
                }
                self.gesture = Gesture::PressedNote {
                    id: note_id,
                    press_screen: self.cursor,
                    ctrl: self.ctrl,
                };
            }
            Hit::Pin { name, kind } => {
                self.gesture = Gesture::PressedPin {
                    kind,
                    name,
                    press_screen: self.cursor,
                };
            }
            Hit::Wire {
                net_id,
                segment_index,
            } => {
                self.gesture = Gesture::PressedWire {
                    net_id,
                    segment_index,
                    press_screen: self.cursor,
                    ctrl: self.ctrl,
                };
            }
            Hit::OutputWire { pin } => {
                if self.ctrl {
                    if let Some(i) = self.selection.pin_wires.iter().position(|p| p == &pin) {
                        self.selection.pin_wires.remove(i);
                    } else {
                        self.selection.pin_wires.push(pin);
                    }
                } else {
                    self.selection.clear();
                    self.selection.pin_wires = vec![pin];
                }
            }
            Hit::Empty => {
                self.gesture = Gesture::PressedEmpty {
                    press_screen: self.cursor,
                };
            }
        }
    }

    /// A click while drawing a wire. Every outcome commits to the document
    /// right away: a port or node finishes this stretch of wire, empty
    /// space drops a new node and drawing continues from it.
    fn wire_click(&mut self) {
        let Gesture::DrawingWire { anchor } = &self.gesture else {
            return;
        };
        let anchor = anchor.clone();
        match self.wire_target() {
            WireTarget::Port(target) => {
                let label = format!(
                    "Connected to {}.{}",
                    target.component_id, target.port_name
                );
                match self.editor.add_wire(anchor, WirePoint::Port(target)) {
                    Ok(_) => {
                        self.gesture = Gesture::Idle;
                        self.info(label);
                    }
                    Err(EditError::NoopWire) => {}
                    Err(e) => self.error(e.to_string()),
                }
            }
            WireTarget::Node {
                net_id,
                junction_id,
            } => {
                match self.editor.add_wire(
                    anchor,
                    WirePoint::Junction {
                        net_id,
                        junction_id,
                    },
                ) {
                    Ok(_) => {
                        self.gesture = Gesture::Idle;
                        self.info("Joined the wires");
                    }
                    Err(EditError::NoopWire) => {}
                    Err(e) => self.error(e.to_string()),
                }
            }
            WireTarget::Incompatible => {
                self.error("This wire already has an output driving it");
            }
            WireTarget::Free(point) => {
                match self.editor.add_wire(anchor, WirePoint::New(point)) {
                    Ok(attach) => {
                        self.gesture = Gesture::DrawingWire {
                            anchor: WirePoint::Junction {
                                net_id: attach.net_id,
                                junction_id: attach
                                    .new_junction
                                    .expect("New points create a junction"),
                            },
                        };
                    }
                    Err(EditError::NoopWire) => {}
                    Err(e) => self.error(e.to_string()),
                }
            }
        }
    }

    pub fn left_released(&mut self) {
        match std::mem::replace(&mut self.gesture, Gesture::Idle) {
            Gesture::Idle => {}
            Gesture::Panning { .. } => {}
            gesture @ Gesture::Placing { .. } => {
                // Placement acts on press; the gesture persists (Shift kept
                // it alive or the press failed and the user gets to retry).
                self.gesture = gesture;
            }
            Gesture::PressedComponent { id, ctrl, .. } => {
                if ctrl {
                    // Ctrl-click: toggle in/out of the multi-selection.
                    if let Some(i) = self.selection.components.iter().position(|c| c == &id) {
                        self.selection.components.remove(i);
                    } else {
                        self.selection.components.push(id);
                    }
                    self.selection.pin = None;
                } else {
                    // A click (no drag): collapse the selection to this one.
                    self.selection.clear();
                    self.selection.components = vec![id];
                }
            }
            Gesture::PressedNote { id, ctrl, .. } => {
                if ctrl {
                    if let Some(i) = self.selection.notes.iter().position(|n| n == &id) {
                        self.selection.notes.remove(i);
                    } else {
                        self.selection.notes.push(id);
                    }
                    self.selection.pin = None;
                } else {
                    self.selection.clear();
                    self.selection.notes = vec![id];
                }
            }
            Gesture::DraggingComponents {
                ids,
                note_ids,
                delta,
                copying,
                ..
            } => {
                if copying {
                    if delta != [0, 0] {
                        let fragment = self.editor.fragment(&ids, &note_ids);
                        match self.editor.paste_fragment(&fragment, delta, false) {
                            Ok((new_ids, new_notes)) => {
                                self.selection.clear();
                                self.selection.components = new_ids;
                                self.selection.notes = new_notes;
                                self.info("Copied");
                            }
                            Err(e) => self.error(e.to_string()),
                        }
                    }
                } else {
                    match self.editor.move_items(&ids, &note_ids, delta) {
                        Ok(()) => {}
                        Err(e) => self.error(e.to_string()),
                    }
                }
            }
            Gesture::PressedEmpty { .. } => {
                self.selection.clear();
            }
            Gesture::BandSelecting { start_grid } => {
                let g = self.cursor_grid();
                let band = rect_between(start_grid, g);
                let abstractions = &self.editor.project.abstractions;
                let (components, _) = self.editor.sheet_data();
                let components: Vec<String> = components
                    .iter()
                    .filter(|c| layout::footprint(abstractions, c).intersects(&band))
                    .map(|c| c.id.clone())
                    .collect();
                let notes: Vec<String> = self
                    .editor
                    .sheet_notes()
                    .iter()
                    .filter(|n| layout::note_bounds(n).intersects(&band))
                    .map(|n| n.id.clone())
                    .collect();
                // Wire segments come along when both their endpoints are in
                // the band — wires select like anything else.
                let (sheet_components, wire_nets) = self.editor.sheet_data();
                let mut segments: Vec<(String, usize)> = Vec::new();
                for net in wire_nets {
                    for (i, seg) in net.segments.iter().enumerate() {
                        let ends = [&seg.from, &seg.to].map(|e| {
                            layout::resolve_endpoint(
                                abstractions,
                                sheet_components,
                                self.interface(),
                                &net.junctions,
                                e,
                            )
                        });
                        if let [Some(a), Some(b)] = ends {
                            if band.contains(a) && band.contains(b) {
                                segments.push((net.id.clone(), i));
                            }
                        }
                    }
                }
                self.selection.clear();
                self.selection.components = components;
                self.selection.notes = notes;
                self.selection.segments = segments;
            }
            gesture @ Gesture::DrawingWire { .. } => {
                // Drag-style wiring: releasing over a port or node commits.
                self.gesture = gesture;
                if matches!(
                    self.wire_target(),
                    WireTarget::Port(_) | WireTarget::Node { .. }
                ) {
                    self.wire_click();
                }
            }
            Gesture::PressedJunction {
                net_id,
                junction_id,
                ..
            } => {
                // A click on a node: start drawing a wire from it (branch).
                self.gesture = Gesture::DrawingWire {
                    anchor: WirePoint::Junction {
                        net_id,
                        junction_id,
                    },
                };
                self.info(
                    "Wiring from the node — click a port to connect, empty space for another node",
                );
            }
            Gesture::DraggingJunction {
                net_id,
                junction_id,
                pos,
            } => match self.editor.move_junction(&net_id, &junction_id, pos) {
                Ok(()) => {}
                Err(e) => self.error(e.to_string()),
            },
            Gesture::PressedWire {
                net_id,
                segment_index,
                ctrl,
                ..
            } => {
                let key = (net_id, segment_index);
                if ctrl {
                    if let Some(i) = self.selection.segments.iter().position(|s| s == &key) {
                        self.selection.segments.remove(i);
                    } else {
                        self.selection.segments.push(key);
                    }
                } else {
                    self.selection.clear();
                    self.selection.segments = vec![key];
                }
            }
            Gesture::PressedPin { kind, name, .. } => {
                // A click: select the pin.
                self.selection.clear();
                self.selection.pin = Some((kind, name.clone()));
                self.info(format!(
                    "Pin \"{name}\" — drag moves it, double-click renames, Delete removes"
                ));
            }
            Gesture::DraggingPin { name, pos } => {
                match self.editor.set_pin_position(&name, pos) {
                    Ok(()) => {}
                    Err(e) => self.error(e.to_string()),
                }
            }
        }
    }

    pub fn middle_or_right_pressed(&mut self) {
        if self.prompt.is_some() || self.layout_dialog.is_some() {
            return;
        }
        // An active constructive gesture? Right/middle click ends it.
        match self.gesture {
            Gesture::Placing { .. } => {
                self.gesture = Gesture::Idle;
                self.info("Cancelled");
            }
            Gesture::DrawingWire { .. } => {
                // Everything placed so far is already committed; this just
                // puts the pencil down.
                self.gesture = Gesture::Idle;
                self.info("Stopped wiring — click a node to continue from it");
            }
            _ => {
                self.gesture = Gesture::Panning { last: self.cursor };
            }
        }
    }

    pub fn pan_released(&mut self) {
        if matches!(self.gesture, Gesture::Panning { .. }) {
            self.gesture = Gesture::Idle;
        }
    }

    pub fn scrolled(&mut self, amount: f32) {
        if self.prompt.is_some() || self.layout_dialog.is_some() {
            return;
        }
        // Over the palette, the wheel scrolls the palette itself.
        if self.cursor[0] < theme::PALETTE_WIDTH && self.cursor[1] > theme::TOPBAR_HEIGHT {
            let content = 58.0 + self.palette_items().len() as f32 * 36.0 + 34.0;
            let available = self.window_size[1] - theme::TOPBAR_HEIGHT - theme::STATUSBAR_HEIGHT;
            let max_scroll = (content - available).max(0.0);
            self.palette_scroll = (self.palette_scroll - amount * 0.4).clamp(0.0, max_scroll);
            return;
        }
        if !self.canvas_rect().contains(self.cursor) {
            return;
        }
        let factor = (1.0015f32).powf(amount);
        self.camera.zoom_at(self.cursor, factor);
        self.update_focus();
        self.recompute_hover();
    }

    pub fn double_clicked(&mut self) {
        if !self.canvas_rect().contains(self.cursor) || self.is_running() {
            return;
        }
        if let Hit::Note { note_id } = self.hover.clone() {
            let text = self
                .editor
                .sheet_notes()
                .iter()
                .find(|n| n.id == note_id)
                .map(|n| n.text.clone())
                .unwrap_or_default();
            self.prompt = Some(Prompt {
                title: "Edit note (empty text removes it)".into(),
                value: text,
                action: PromptAction::EditNote { id: note_id },
                identifier: false,
            });
            return;
        }
        if let Hit::Pin { name, kind } = self.hover.clone() {
            self.prompt = Some(Prompt {
                title: format!("Rename pin \"{name}\""),
                value: name.clone(),
                action: PromptAction::RenamePin { kind, old: name },
                identifier: true,
            });
            return;
        }

        if let Hit::Component { component_id } = self.hover.clone() {
            let (components, _) = self.editor.sheet_data();
            let Some(comp) = components.iter().find(|c| c.id == component_id) else {
                return;
            };
            if comp.component_type == "dff" || comp.component_type == "const" {
                match self.editor.toggle_dff_init(&component_id) {
                    Ok(v) => self.info(format!(
                        "{component_id} now starts at {}",
                        if v { "1" } else { "0" }
                    )),
                    Err(e) => self.error(e.to_string()),
                }
                return;
            }
            // Double-click an abstraction instance: jump the zoom inside it.
            let type_name = comp.component_type.clone();
            let instance_id = comp.id.clone();
            if let Some(i) = self
                .editor
                .project
                .abstractions
                .iter()
                .position(|a| a.name == type_name)
            {
                let parent = self.editor.sheet;
                if self.editor.open_sheet(SheetRef::Abstraction(i)).is_ok() {
                    self.path.push(PathSeg {
                        parent,
                        instance_id,
                        child: SheetRef::Abstraction(i),
                    });
                    // Land just past the descend threshold: the content
                    // slightly overfills the screen, so zooming out walks
                    // back out naturally.
                    let abstractions = &self.editor.project.abstractions;
                    let b = diycpu::layout::abstraction_inner_bounds(
                        abstractions,
                        &abstractions[i],
                    );
                    let v = self.viewport_min();
                    let scale = DESCEND_AT * 0.98 * v / b.w.max(b.h).max(0.1);
                    let center = self.canvas_center();
                    self.camera.set_view(crate::camera::View {
                        scale,
                        offset: [
                            center[0] - (b.x + b.w / 2.0) * scale,
                            center[1] - (b.y + b.h / 2.0) * scale,
                        ],
                    });
                    self.selection.clear();
                    self.gesture = Gesture::Idle;
                    self.info(format!(
                        "Inside \"{type_name}\" — zoom out or press Esc to leave"
                    ));
                }
            }
        }
    }

    // ── Palette / toolbar actions (called by the shell) ──────────────────

    pub fn palette_clicked(&mut self, type_name: &str) {
        if !self.require_edit_mode() {
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

    // ── Keyboard ─────────────────────────────────────────────────────────

    pub fn escape(&mut self) {
        if !matches!(self.gesture, Gesture::Idle) {
            self.gesture = Gesture::Idle;
            self.info("Cancelled");
        } else if self.is_running() {
            self.toggle_run();
        } else if !self.selection.is_empty() {
            self.selection.clear();
        } else {
            self.action_back_to_top();
        }
    }

    pub fn delete_selection(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        // A hovered wire node or segment takes priority: point at it and
        // press Delete.
        if let Hit::Junction {
            net_id,
            junction_id,
        } = self.hover.clone()
        {
            match self.editor.delete_junction(&net_id, &junction_id) {
                Ok(()) => self.info("Removed node"),
                Err(e) => self.error(e.to_string()),
            }
            self.recompute_hover();
            return;
        }
        if let Hit::Wire {
            net_id,
            segment_index,
        } = self.hover.clone()
        {
            match self.editor.delete_segment(&net_id, segment_index) {
                Ok(()) => self.info("Removed wire segment"),
                Err(e) => self.error(e.to_string()),
            }
            self.selection.segments.clear();
            self.recompute_hover();
            return;
        }
        if let Some((kind, name)) = self.selection.pin.clone() {
            match self.editor.remove_interface_pin(kind, &name) {
                Ok(()) => {
                    self.selection.pin = None;
                    self.info(format!("Removed pin \"{name}\""));
                }
                Err(e) => self.error(e.to_string()),
            }
        } else if !self.selection.is_empty() {
            let ids = self.selection.components.clone();
            let note_ids = self.selection.notes.clone();
            let segments = self.selection.segments.clone();
            let pin_wires = self.selection.pin_wires.clone();
            let n = ids.len() + note_ids.len() + segments.len() + pin_wires.len();
            match self
                .editor
                .delete_mixed(&ids, &note_ids, &segments, &pin_wires)
            {
                Ok(()) => {
                    self.selection.clear();
                    self.info(if n == 1 {
                        "Deleted".to_string()
                    } else {
                        format!("Deleted {n} items")
                    });
                }
                Err(e) => self.error(e.to_string()),
            }
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
        let fragment = self
            .editor
            .fragment(&self.selection.components, &self.selection.notes);
        if fragment.is_empty() {
            self.info("Nothing selected to copy");
            return;
        }
        let n = fragment.components.len() + fragment.notes.len();
        self.clipboard = Some(fragment);
        self.info(format!("Copied {n} item(s)"));
    }

    pub fn cut_selection(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        let fragment = self
            .editor
            .fragment(&self.selection.components, &self.selection.notes);
        if fragment.is_empty() {
            self.info("Nothing selected to cut");
            return;
        }
        let ids = self.selection.components.clone();
        let note_ids = self.selection.notes.clone();
        match self.editor.delete_items(&ids, &note_ids) {
            Ok(()) => {
                let n = fragment.components.len() + fragment.notes.len();
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
            Ok((new_ids, new_notes)) => {
                self.selection.clear();
                self.selection.components = new_ids;
                self.selection.notes = new_notes;
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
        let fragment = self
            .editor
            .fragment(&self.selection.components, &self.selection.notes);
        if fragment.is_empty() {
            self.info("Nothing selected to duplicate");
            return;
        }
        match self.editor.paste_fragment(&fragment, [1, 1], true) {
            Ok((new_ids, new_notes)) => {
                self.info(format!(
                    "Duplicated {} item(s)",
                    new_ids.len() + new_notes.len()
                ));
                self.selection.clear();
                self.selection.components = new_ids;
                self.selection.notes = new_notes;
            }
            Err(e) => self.error(e.to_string()),
        }
    }

    /// Single entry point for all keyboard input. A live prompt captures
    /// everything; otherwise keys map to app actions.
    pub fn handle_key(&mut self, key: KeyInput) {
        if self.prompt.is_some() {
            self.prompt_key(key);
            return;
        }
        if self.layout_dialog.is_some() {
            if matches!(key, KeyInput::Escape | KeyInput::Enter) {
                self.layout_dialog = None;
            }
            return;
        }
        match key {
            KeyInput::Escape => self.escape(),
            KeyInput::Delete | KeyInput::Backspace => self.delete_selection(),
            KeyInput::Enter => {}
            KeyInput::ArrowRight => self.action_step_forward(),
            KeyInput::ArrowLeft => self.action_step_back(),
            KeyInput::Home => self.action_rewind(),
            KeyInput::Space => self.action_run_toggle(),
            KeyInput::Char(ch) => self.key_char(ch),
        }
    }

    pub fn dismiss_prompt(&mut self) {
        if self.prompt.take().is_some() {
            self.info("Cancelled");
        }
    }

    fn prompt_key(&mut self, key: KeyInput) {
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
                        c.is_ascii_alphanumeric() || c == '_' || c == '-'
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

    fn commit_prompt(&mut self) {
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
                match self.editor.add_interface_pin(kind, &name) {
                    Ok(()) => self.info(format!("Added pin \"{name}\"")),
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
                match self.editor.rename_interface_pin(kind, &old, &name) {
                    Ok(()) => self.info(format!("Renamed pin to \"{name}\"")),
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
                PortKind::Input => "Name the input pin".into(),
                PortKind::Output => "Name the output pin".into(),
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

    pub fn action_back_to_top(&mut self) {
        if self.ascend_once() {
            self.info("Zoomed out one level");
        } else if self.view_root != SheetRef::Top {
            self.view_root = SheetRef::Top;
            let _ = self.editor.open_sheet(SheetRef::Top);
            self.selection.clear();
            self.gesture = Gesture::Idle;
            self.pending_fit = true;
            self.info("Back to the main circuit");
        }
    }

    fn key_char(&mut self, ch: &str) {
        let lower = ch.to_lowercase();
        match (lower.as_str(), self.ctrl, self.shift) {
            ("z", true, false) => self.undo(),
            ("z", true, true) => self.redo(),
            ("y", true, _) => self.redo(),
            ("s", true, false) => self.save(),
            ("s", true, true) => {
                if self.require_edit_mode() {
                    self.save_as()
                }
            }
            ("o", true, _) => {
                if self.require_edit_mode() {
                    self.open()
                }
            }
            ("n", true, _) => {
                if self.require_edit_mode() {
                    self.new_project()
                }
            }
            ("d", true, _) => self.duplicate(),
            ("c", true, _) => self.copy_selection(),
            ("x", true, _) => self.cut_selection(),
            ("v", true, _) => self.paste_clipboard(),
            ("p", false, _) => self.action_play_toggle(),
            ("f", false, _) => {
                self.fit_view();
                self.info("View fitted to circuit");
            }
            ("0", true, _) => {
                let center = self.canvas_center();
                let factor = 1.0 / self.camera.zoom;
                self.camera.zoom_at(center, factor);
                self.update_focus();
                self.recompute_hover();
            }
            _ => {}
        }
    }

    pub fn focus_lost(&mut self) {
        // Mid-gesture state must never survive without live input.
        if !matches!(self.gesture, Gesture::Placing { .. }) {
            self.gesture = Gesture::Idle;
        }
        self.ctrl = false;
        self.shift = false;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DiscardIntent {
    New,
    Open,
    Close,
}

pub enum WireTarget {
    /// A compatible port to commit to.
    Port(PortRef),
    /// An existing wire node to join.
    Node { net_id: String, junction_id: String },
    /// Hovering a port or node that can't take this wire.
    Incompatible,
    /// Free space (snapped to half-grid) — a node would go here.
    Free([f32; 2]),
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

fn snap_half(g: [f32; 2]) -> [f32; 2] {
    [(g[0] * 2.0).round() / 2.0, (g[1] * 2.0).round() / 2.0]
}

fn rect_between(a: [f32; 2], b: [f32; 2]) -> Rect {
    Rect {
        x: a[0].min(b[0]),
        y: a[1].min(b[1]),
        w: (a[0] - b[0]).abs(),
        h: (a[1] - b[1]).abs(),
    }
}

pub fn load_project(path: &std::path::Path) -> Result<(Project, Vec<String>), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let mut project: Project = serde_json::from_str(&text)
        .map_err(|e| format!("Could not parse {}: {e}", path.display()))?;
    let notes = diycpu::editor::normalize(&mut project);
    Ok((project, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the app the way a user would: cursor moves and clicks at grid
    /// positions, straight through the real gesture state machine.
    pub struct Driver {
        pub app: App,
    }

    impl Driver {
        pub fn new() -> Self {
            let mut app = App::new(None);
            app.window_size = [1280.0, 720.0];
            app.pending_fit = false;
            // Park the origin comfortably inside the canvas.
            app.camera.offset = [400.0, 200.0];
            Driver { app }
        }

        pub fn move_to(&mut self, g: [f32; 2]) {
            let s = self.app.camera.grid_to_screen(g);
            self.app.mouse_moved(s);
        }

        pub fn click(&mut self, g: [f32; 2]) {
            self.move_to(g);
            self.app.left_pressed();
            self.app.left_released();
            // The shell recomputes hover every frame; mimic that.
            self.app.recompute_hover();
        }

        pub fn drag(&mut self, from: [f32; 2], to: [f32; 2]) {
            self.move_to(from);
            self.app.left_pressed();
            // Two moves so the drag threshold is crossed mid-gesture.
            self.move_to([(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0]);
            self.move_to(to);
            self.app.left_released();
            self.app.recompute_hover();
        }

        pub fn place(&mut self, type_name: &str, center: [f32; 2]) {
            self.app.palette_clicked(type_name);
            self.click(center);
        }

        pub fn type_text(&mut self, s: &str) {
            for c in s.chars() {
                self.app.handle_key(KeyInput::Char(&c.to_string()));
            }
        }
    }

    pub fn port_at(app: &App, comp: &str, port: &str) -> [f32; 2] {
        let (components, _) = app.editor.sheet_data();
        let c = components.iter().find(|c| c.id == comp).unwrap();
        diycpu::layout::port_pos(&app.editor.project.abstractions, c, port)
            .unwrap()
            .pos
    }

    pub fn center_of(app: &App, comp: &str) -> [f32; 2] {
        let (components, _) = app.editor.sheet_data();
        let c = components.iter().find(|c| c.id == comp).unwrap();
        let r = diycpu::layout::footprint(&app.editor.project.abstractions, c);
        [r.x + r.w / 2.0, r.y + r.h / 2.0]
    }

    #[test]
    fn place_move_wire_flow() {
        let mut d = Driver::new();
        d.place("clk", [1.0, 1.0]);
        d.place("dff", [8.0, 1.0]);
        assert_eq!(d.app.editor.project.components.len(), 2);
        assert!(!d.app.status.is_error, "{}", d.app.status.text);

        // Wire clk.out → dff.clk by dragging port to port.
        let from = port_at(&d.app, "clk_1", "out");
        let to = port_at(&d.app, "dff_1", "clk");
        d.drag(from, to);
        assert_eq!(d.app.editor.project.wire_nets.len(), 1);
        assert_eq!(d.app.editor.project.wire_nets[0].sinks.len(), 1);

        // Move the dff; the wire follows because endpoints are symbolic.
        let before = d.app.editor.project.components[1].pos;
        let body = center_of(&d.app, "dff_1");
        d.drag(body, [body[0] + 5.0, body[1] + 2.0]);
        let dff = &d.app.editor.project.components[1];
        assert_eq!(dff.pos, [before[0] + 5.0, before[1] + 2.0]);
        assert_eq!(d.app.editor.project.wire_nets.len(), 1);
    }

    #[test]
    fn click_click_wire_with_bend() {
        let mut d = Driver::new();
        d.place("nand", [0.0, 0.0]);
        d.place("nand", [10.0, 4.0]);

        let from = port_at(&d.app, "nand_1", "out");
        let to = port_at(&d.app, "nand_2", "a");
        d.click(from); // start wire
        assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
        d.click([6.0, 6.0]); // bend on empty canvas
        assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
        d.click(to); // finish on the input port
        assert!(matches!(d.app.gesture, Gesture::Idle));

        let net = &d.app.editor.project.wire_nets[0];
        assert_eq!(net.junctions.len(), 1);
        assert_eq!(net.junctions[0].pos, [6.0, 6.0]);
        assert_eq!(net.segments.len(), 2);
    }

    #[test]
    fn wires_are_placed_objects_escape_keeps_them() {
        let mut d = Driver::new();
        d.place("nand", [0.0, 0.0]);

        // out → node → node, then stop: everything clicked stays placed.
        d.click(port_at(&d.app, "nand_1", "out"));
        d.click([6.0, 6.0]);
        d.click([8.0, 2.0]);
        assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
        d.app.handle_key(KeyInput::Escape);
        assert!(matches!(d.app.gesture, Gesture::Idle));
        let net = &d.app.editor.project.wire_nets[0];
        assert_eq!(net.junctions.len(), 2);
        assert_eq!(net.segments.len(), 2);
        assert_eq!(net.sinks.len(), 0);

        // Undo peels the wire back click by click.
        d.app.undo();
        assert_eq!(d.app.editor.project.wire_nets[0].junctions.len(), 1);
        d.app.undo();
        assert!(d.app.editor.project.wire_nets.is_empty());
    }

    #[test]
    fn stopped_wire_resumes_from_its_node_from_either_side() {
        let mut d = Driver::new();
        d.place("nand", [0.0, 0.0]);
        d.place("nand", [10.0, 6.0]);

        // Output → node, stop drawing (the stub stays).
        d.click(port_at(&d.app, "nand_1", "out"));
        d.click([6.0, 6.0]);
        d.app.middle_or_right_pressed();
        assert!(matches!(d.app.gesture, Gesture::Idle));
        assert_eq!(d.app.editor.project.wire_nets[0].sinks.len(), 0);

        // Now start from the INPUT and finish on the stub's node.
        d.click(port_at(&d.app, "nand_2", "a"));
        assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
        d.click([6.0, 6.0]); // the node lives here
        assert!(matches!(d.app.gesture, Gesture::Idle));
        let net = &d.app.editor.project.wire_nets[0];
        assert_eq!(net.source, Some(PortRef {
            component_id: "nand_1".into(),
            port_name: "out".into(),
        }));
        assert_eq!(net.sinks.len(), 1);
    }

    #[test]
    fn branch_from_a_mid_wire_node() {
        let mut d = Driver::new();
        d.place("nand", [0.0, 0.0]);
        d.place("nand", [10.0, 0.0]);
        d.place("nand", [10.0, 6.0]);

        // out → node → nand_2.a
        d.click(port_at(&d.app, "nand_1", "out"));
        d.click([6.0, 3.0]);
        d.click(port_at(&d.app, "nand_2", "a"));
        assert!(matches!(d.app.gesture, Gesture::Idle));

        // Click the node to branch, land on nand_3.a.
        d.click([6.0, 3.0]);
        assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
        d.click(port_at(&d.app, "nand_3", "a"));
        let net = &d.app.editor.project.wire_nets[0];
        assert_eq!(net.sinks.len(), 2);
        assert_eq!(net.junctions.len(), 1);
        assert_eq!(net.segments.len(), 3);
    }

    #[test]
    fn drag_moves_a_node_and_delete_removes_it() {
        let mut d = Driver::new();
        d.place("nand", [0.0, 0.0]);
        d.click(port_at(&d.app, "nand_1", "out"));
        d.click([6.0, 3.0]);
        d.app.middle_or_right_pressed(); // stop drawing

        // Drag the node somewhere else.
        d.drag([6.0, 3.0], [9.0, 5.0]);
        assert_eq!(d.app.editor.project.wire_nets[0].junctions[0].pos, [9.0, 5.0]);

        // Hover it and press Delete: node and its wire go away.
        d.move_to([9.0, 5.0]);
        d.app.handle_key(KeyInput::Delete);
        assert!(d.app.editor.project.wire_nets.is_empty());
    }

    #[test]
    fn invalid_drop_reports_and_leaves_document_alone() {
        let mut d = Driver::new();
        d.place("nand", [1.0, 1.0]);
        d.place("nand", [7.0, 1.0]);
        let before = d.app.editor.project.components[0].pos;
        // Try to drop nand_1 onto nand_2.
        d.drag(center_of(&d.app, "nand_1"), center_of(&d.app, "nand_2"));
        assert!(d.app.status.is_error);
        assert_eq!(d.app.editor.project.components[0].pos, before);
    }

    #[test]
    fn band_select_and_delete() {
        let mut d = Driver::new();
        d.place("nand", [1.0, 1.0]); // top-left corner lands at (0, 0)
        d.place("nand", [7.0, 1.0]);
        d.place("nand", [1.0, 7.0]);
        d.drag([-2.0, -2.0], [11.0, 4.0]); // band over the top two
        assert_eq!(d.app.selection.components.len(), 2);
        d.app.handle_key(KeyInput::Delete);
        assert_eq!(d.app.editor.project.components.len(), 1);
        assert_eq!(d.app.editor.project.components[0].id, "nand_3");
    }

    #[test]
    fn run_mode_locks_editing() {
        let mut d = Driver::new();
        d.place("dff", [0.0, 0.0]);
        d.app.action_run_toggle();
        assert!(d.app.is_running());

        d.app.palette_clicked("nand");
        assert!(matches!(d.app.gesture, Gesture::Idle));
        d.app.handle_key(KeyInput::Delete);
        assert_eq!(d.app.editor.project.components.len(), 1);

        d.app.handle_key(KeyInput::ArrowRight);
        // Esc exits run mode even with a live selection.
        d.click(center_of(&d.app, "dff_1"));
        d.app.handle_key(KeyInput::Escape);
        assert!(!d.app.is_running());
    }

    #[test]
    fn autoplay_advances_and_manual_steps_pause() {
        let mut d = Driver::new();
        d.place("clk", [1.0, 1.0]);
        // P from edit mode: starts running AND playing.
        d.app.handle_key(KeyInput::Char("p"));
        assert!(d.app.is_running());
        assert!(d.app.playing);

        d.app.play_tick();
        d.app.play_tick();
        let Mode::Run(session) = &d.app.mode else { unreachable!() };
        assert_eq!(session.cursor, 2);

        // A manual step pauses playback.
        d.app.handle_key(KeyInput::ArrowLeft);
        assert!(!d.app.playing);
        let Mode::Run(session) = &d.app.mode else { unreachable!() };
        assert_eq!(session.cursor, 1);
        // Ticks do nothing while paused.
        d.app.play_tick();
        let Mode::Run(session) = &d.app.mode else { unreachable!() };
        assert_eq!(session.cursor, 1);

        // P resumes; stopping the run stops playback too.
        d.app.handle_key(KeyInput::Char("p"));
        assert!(d.app.playing);
        d.app.handle_key(KeyInput::Escape);
        assert!(!d.app.is_running());
        assert!(!d.app.playing);
    }

    #[test]
    fn undo_mid_gesture_is_safe() {
        let mut d = Driver::new();
        d.place("nand", [0.0, 0.0]);
        d.place("nand", [8.0, 0.0]);
        d.click(port_at(&d.app, "nand_1", "out")); // start a wire
        d.app.undo(); // undo while drawing
        assert!(matches!(d.app.gesture, Gesture::Idle));
        assert_eq!(d.app.editor.project.components.len(), 1);
    }

    #[test]
    fn build_abstraction_end_to_end_through_gestures() {
        let mut d = Driver::new();

        // Create the abstraction via the prompt.
        d.app.action_new_abstraction();
        d.type_text("inverter");
        d.app.handle_key(KeyInput::Enter);
        assert_eq!(d.app.editor.project.abstractions.len(), 1);
        assert!(matches!(d.app.editor.sheet, SheetRef::Abstraction(0)));

        // Add pins via prompts.
        d.app.action_add_pin(PortKind::Input);
        d.type_text("x");
        d.app.handle_key(KeyInput::Enter);
        d.app.action_add_pin(PortKind::Output);
        d.type_text("q");
        d.app.handle_key(KeyInput::Enter);

        // Place a nand and wire pin → gate → pin.
        d.place("nand", [1.5, 1.0]);
        let abstractions = &d.app.editor.project.abstractions;
        let def = &abstractions[0];
        let pins = diycpu::layout::interface_pins(abstractions, def);
        let in_pin = pins.iter().find(|p| p.name == "x").unwrap().pos;
        let out_pin = pins.iter().find(|p| p.name == "q").unwrap().pos;

        d.drag(in_pin, port_at(&d.app, "nand_1", "a"));
        d.drag(in_pin, port_at(&d.app, "nand_1", "b"));
        d.drag(port_at(&d.app, "nand_1", "out"), out_pin);

        let def = &d.app.editor.project.abstractions[0];
        assert_eq!(def.wire_nets.len(), 1);
        assert_eq!(def.wire_nets[0].sinks.len(), 2);
        assert_eq!(def.outputs.get("q"), Some(&"nand_1".to_string()));

        // Back to top (first Esc clears the selection, second navigates),
        // place an instance, feed it a constant 1, run.
        d.app.handle_key(KeyInput::Escape);
        d.app.handle_key(KeyInput::Escape);
        assert_eq!(d.app.editor.sheet, SheetRef::Top);
        d.place("dff", [1.0, 1.0]);
        d.app.double_clicked(); // cursor still over the dff → init = 1
        assert_eq!(d.app.editor.project.components[0].init, Some(true));
        d.place("inverter", [9.0, 1.0]);
        d.drag(
            port_at(&d.app, "dff_1", "out"),
            port_at(&d.app, "inverter_1", "x"),
        );

        d.app.action_run_toggle();
        assert!(d.app.is_running(), "compile failed: {}", d.app.status.text);
        let Mode::Run(session) = &d.app.mode else { unreachable!() };
        // NOT(1) = 0 on the instance output.
        assert_eq!(
            session.port_value_at(
                &[],
                false,
                &PortRef {
                    component_id: "inverter_1".into(),
                    port_name: "q".into()
                }
            ),
            Some(false)
        );
    }
}

#[cfg(test)]
mod crud_tests {
    use super::tests::*;
    use super::*;

    #[test]
    fn note_lifecycle_through_gestures() {
        let mut d = Driver::new();
        // Place via palette + prompt.
        d.app.palette_clicked("note");
        d.click([4.0, 4.0]);
        assert!(d.app.prompt.is_some());
        d.type_text("hello world");
        d.app.handle_key(KeyInput::Space);
        d.type_text("note");
        d.app.handle_key(KeyInput::Enter);
        assert_eq!(d.app.editor.project.notes.len(), 1);
        assert_eq!(d.app.editor.project.notes[0].text, "hello world note");

        // Drag it somewhere else.
        let bounds = diycpu::layout::note_bounds(&d.app.editor.project.notes[0]);
        let center = [bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0];
        d.drag(center, [center[0] + 5.0, center[1] + 3.0]);
        let moved = &d.app.editor.project.notes[0];
        assert_eq!(moved.pos[0], bounds.x + 5.0);

        // Double-click opens the editor prefilled; emptying removes it.
        let bounds = diycpu::layout::note_bounds(&d.app.editor.project.notes[0]);
        d.move_to([bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0]);
        d.app.recompute_hover();
        d.app.double_clicked();
        assert!(d.app.prompt.is_some());
        let len = d.app.prompt.as_ref().unwrap().value.chars().count();
        for _ in 0..len {
            d.app.handle_key(KeyInput::Backspace);
        }
        d.app.handle_key(KeyInput::Enter);
        assert!(d.app.editor.project.notes.is_empty());
    }

    #[test]
    fn rename_pin_via_double_click() {
        let mut d = Driver::new();
        d.app.action_new_abstraction();
        d.type_text("thing");
        d.app.handle_key(KeyInput::Enter);
        d.app.action_add_pin(PortKind::Input);
        d.type_text("x");
        d.app.handle_key(KeyInput::Enter);

        // Pin rails sit left of the content — pan them into the canvas.
        d.app.camera.offset = [700.0, 300.0];

        // Double-click the pin's name plate.
        let abstractions = &d.app.editor.project.abstractions;
        let pin = diycpu::layout::interface_pins(abstractions, &abstractions[0])
            .into_iter()
            .find(|p| p.name == "x")
            .unwrap();
        let plate = pin.plate;
        d.move_to([plate.x + plate.w / 2.0, plate.y + plate.h / 2.0]);
        d.app.recompute_hover();
        d.app.double_clicked();
        assert!(d.app.prompt.is_some());
        assert_eq!(d.app.prompt.as_ref().unwrap().value, "x");
        d.type_text("_in"); // appended to the prefill
        d.app.handle_key(KeyInput::Enter);
        assert_eq!(d.app.editor.project.abstractions[0].inputs, vec!["x_in"]);
    }

    #[test]
    fn rename_project_and_abstraction_via_title() {
        let mut d = Driver::new();
        d.app.action_rename_title();
        assert!(d.app.prompt.is_some());
        let len = d.app.prompt.as_ref().unwrap().value.chars().count();
        for _ in 0..len {
            d.app.handle_key(KeyInput::Backspace);
        }
        d.type_text("my");
        d.app.handle_key(KeyInput::Space);
        d.type_text("cpu");
        d.app.handle_key(KeyInput::Enter);
        assert_eq!(d.app.editor.project.name, "my cpu");

        d.app.action_new_abstraction();
        d.type_text("adder");
        d.app.handle_key(KeyInput::Enter);
        d.app.action_rename_title(); // now renames the open abstraction
        let len = d.app.prompt.as_ref().unwrap().value.chars().count();
        for _ in 0..len {
            d.app.handle_key(KeyInput::Backspace);
        }
        d.type_text("adder2");
        d.app.handle_key(KeyInput::Enter);
        assert_eq!(d.app.editor.project.abstractions[0].name, "adder2");

        // And delete it from its own sheet.
        d.app.action_delete_abstraction();
        assert!(d.app.editor.project.abstractions.is_empty());
        assert_eq!(d.app.editor.sheet, SheetRef::Top);
    }
}

#[cfg(test)]
mod zoom_tests {
    use super::tests::*;
    use super::*;

    fn build_project_with_instance(d: &mut Driver) {
        d.app.action_new_abstraction();
        d.type_text("box");
        d.app.handle_key(KeyInput::Enter);
        d.place("nand", [2.0, 1.0]);
        d.app.handle_key(KeyInput::Escape); // clear selection
        d.app.handle_key(KeyInput::Escape); // back to top
        assert_eq!(d.app.editor.sheet, SheetRef::Top);
        d.place("box", [5.0, 5.0]);
        d.app.handle_key(KeyInput::Escape);
    }

    /// Aim the camera so the given grid rect fills `fill`× the viewport,
    /// centered.
    fn aim(d: &mut Driver, rect: diycpu::layout::Rect, fill: f32) {
        let cr = d.app.canvas_rect();
        let v = cr.w.min(cr.h);
        let scale = fill * v / rect.w.max(rect.h);
        d.app.camera.zoom = scale / crate::camera::CELL;
        d.app.camera.offset = [
            cr.x + cr.w / 2.0 - (rect.x + rect.w / 2.0) * scale,
            cr.y + cr.h / 2.0 - (rect.y + rect.h / 2.0) * scale,
        ];
    }

    #[test]
    fn zooming_in_descends_and_out_ascends_seamlessly() {
        let mut d = Driver::new();
        build_project_with_instance(&mut d);
        let abstractions = &d.app.editor.project.abstractions;
        let comp = &d.app.editor.project.components[0];
        let rect = diycpu::layout::footprint(abstractions, comp);

        // Big enough and centered: focus descends.
        aim(&mut d, rect, 1.5);
        let view_before = d.app.camera.view();
        let world_probe = [rect.x + 1.0, rect.y + 1.0]; // a point on the parent sheet
        let screen_before = view_before.to_screen(world_probe);

        d.app.update_focus();
        assert_eq!(d.app.editor.sheet, SheetRef::Abstraction(0));
        assert_eq!(d.app.path.len(), 1);

        // Seamless: the same world point still lands on the same pixel when
        // mapped through the rebased camera and the inverse of the map.
        let seg = d.app.path.last().unwrap().clone();
        let map = d.app.seg_map(&seg).unwrap();
        let parent_view = d.app.camera.view().through(&map.inverse());
        let screen_after = parent_view.to_screen(world_probe);
        assert!(
            (screen_before[0] - screen_after[0]).abs() < 0.5
                && (screen_before[1] - screen_after[1]).abs() < 0.5,
            "rebase moved the world: {screen_before:?} vs {screen_after:?}"
        );

        // Zoom the camera way out (in child coordinates): focus ascends.
        d.app.camera.zoom *= 0.2;
        d.app.update_focus();
        assert_eq!(d.app.editor.sheet, SheetRef::Top);
        assert!(d.app.path.is_empty());
    }

    #[test]
    fn escape_ascends_one_level_and_editing_targets_focus() {
        let mut d = Driver::new();
        build_project_with_instance(&mut d);
        let rect = diycpu::layout::footprint(
            &d.app.editor.project.abstractions,
            &d.app.editor.project.components[0],
        );
        aim(&mut d, rect, 1.5);
        d.app.update_focus();
        assert_eq!(d.app.path.len(), 1);

        // Editing while focused edits the abstraction's sheet.
        d.app.camera.zoom = 1.0; // gestures at sane scale (no auto-ascend: no scroll)
        d.app.camera.offset = [400.0, 200.0];
        d.place("dff", [10.0, 1.0]);
        assert_eq!(d.app.editor.project.abstractions[0].components.len(), 2);
        assert!(!d.app.status.is_error, "{}", d.app.status.text);

        d.app.handle_key(KeyInput::Escape); // clear selection
        d.app.handle_key(KeyInput::Escape); // ascend
        assert!(d.app.path.is_empty());
        assert_eq!(d.app.editor.sheet, SheetRef::Top);
    }

    #[test]
    fn breadcrumb_tracks_the_path() {
        let mut d = Driver::new();
        build_project_with_instance(&mut d);
        assert_eq!(d.app.breadcrumb(), vec!["untitled"]);
        let rect = diycpu::layout::footprint(
            &d.app.editor.project.abstractions,
            &d.app.editor.project.components[0],
        );
        aim(&mut d, rect, 1.5);
        d.app.update_focus();
        assert_eq!(d.app.breadcrumb(), vec!["untitled", "box"]);
        d.app.open_as_root(0);
        assert_eq!(d.app.breadcrumb(), vec!["untitled", "box"]);
        assert!(d.app.path.is_empty());
    }

    #[test]
    fn undo_across_sheets_resets_the_view() {
        let mut d = Driver::new();
        build_project_with_instance(&mut d);
        let rect = diycpu::layout::footprint(
            &d.app.editor.project.abstractions,
            &d.app.editor.project.components[0],
        );
        aim(&mut d, rect, 1.5);
        d.app.update_focus();
        assert_eq!(d.app.path.len(), 1);
        // Undo jumps back to a top-sheet edit: view must follow.
        d.app.undo();
        assert_eq!(d.app.editor.sheet, d.app.focus_sheet());
    }
}

// ── Layout dialog ────────────────────────────────────────────────────────────

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
}

#[cfg(test)]
mod clipboard_tests {
    use super::tests::*;
    use super::*;

    #[test]
    fn copy_paste_via_keyboard() {
        let mut d = Driver::new();
        d.place("nand", [1.0, 1.0]);
        d.place("dff", [8.0, 1.0]);
        // Wire them, band-select everything, copy.
        d.drag(port_at(&d.app, "nand_1", "out"), port_at(&d.app, "dff_1", "d"));
        d.drag([-4.0, -3.0], [12.0, 4.0]);
        assert_eq!(d.app.selection.components.len(), 2);
        d.app.ctrl = true;
        d.app.handle_key(KeyInput::Char("c"));
        d.app.ctrl = false;
        assert!(d.app.clipboard.is_some());

        // Paste at a spot lower on the canvas.
        d.move_to([6.0, 12.0]);
        d.app.ctrl = true;
        d.app.handle_key(KeyInput::Char("v"));
        d.app.ctrl = false;
        assert_eq!(d.app.editor.project.components.len(), 4);
        assert_eq!(d.app.editor.project.wire_nets.len(), 2, "wiring came along");
        assert_eq!(d.app.selection.components.len(), 2, "copies are selected");
        assert!(!d.app.status.is_error, "{}", d.app.status.text);
    }

    #[test]
    fn cut_removes_and_paste_restores() {
        let mut d = Driver::new();
        d.place("nand", [1.0, 1.0]);
        d.click(center_of(&d.app, "nand_1"));
        d.app.ctrl = true;
        d.app.handle_key(KeyInput::Char("x"));
        d.app.ctrl = false;
        assert!(d.app.editor.project.components.is_empty());
        d.move_to([5.0, 5.0]);
        d.app.ctrl = true;
        d.app.handle_key(KeyInput::Char("v"));
        d.app.ctrl = false;
        assert_eq!(d.app.editor.project.components.len(), 1);
    }

    #[test]
    fn ctrl_drag_copies_and_ctrl_click_still_toggles() {
        let mut d = Driver::new();
        d.place("nand", [1.0, 1.0]);
        d.place("nand", [8.0, 1.0]);

        // Ctrl-click toggles selection (no drag).
        d.click(center_of(&d.app, "nand_1"));
        d.app.ctrl = true;
        d.click(center_of(&d.app, "nand_2"));
        assert_eq!(d.app.selection.components.len(), 2);
        d.click(center_of(&d.app, "nand_2"));
        assert_eq!(d.app.selection.components.len(), 1);
        d.app.ctrl = false;

        // Ctrl-drag copies the selection to the drop point.
        let original_pos = d.app.editor.project.components[0].pos;
        let from = center_of(&d.app, "nand_1");
        d.app.ctrl = true;
        d.drag(from, [from[0], from[1] + 6.0]);
        d.app.ctrl = false;
        assert_eq!(d.app.editor.project.components.len(), 3);
        let copy_id = &d.app.selection.components[0];
        let copy = d
            .app
            .editor
            .project
            .components
            .iter()
            .find(|c| &c.id == copy_id)
            .unwrap();
        assert_eq!(
            copy.pos,
            [original_pos[0], original_pos[1] + 6.0],
            "copy landed at the drop point"
        );
        // Original still where it was.
        assert_eq!(d.app.editor.project.components[0].pos, original_pos);
    }
}

#[cfg(test)]
mod wire_selection_tests {
    use super::tests::*;
    use super::*;

    fn wired_pair(d: &mut Driver) {
        d.place("nand", [1.0, 1.0]);
        d.place("nand", [9.0, 5.0]);
        d.drag(port_at(&d.app, "nand_1", "out"), port_at(&d.app, "nand_2", "a"));
        assert_eq!(d.app.editor.project.wire_nets.len(), 1);
        d.app.handle_key(KeyInput::Escape); // clear selection
    }

    fn segment_mid(d: &Driver) -> [f32; 2] {
        let a = port_at(&d.app, "nand_1", "out");
        let b = port_at(&d.app, "nand_2", "a");
        [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
    }

    #[test]
    fn click_selects_a_segment_and_delete_removes_it() {
        let mut d = Driver::new();
        wired_pair(&mut d);
        d.click(segment_mid(&d));
        assert_eq!(d.app.selection.segments.len(), 1);
        // Move the cursor off the wire so hover-delete doesn't shortcut.
        d.move_to([0.0, 12.0]);
        d.app.handle_key(KeyInput::Delete);
        assert!(d.app.editor.project.wire_nets.is_empty());
    }

    #[test]
    fn dragging_a_wire_tears_out_a_node_in_one_motion() {
        let mut d = Driver::new();
        wired_pair(&mut d);
        let mid = segment_mid(&d);
        d.drag(mid, [mid[0], mid[1] + 4.0]);
        let net = &d.app.editor.project.wire_nets[0];
        assert_eq!(net.junctions.len(), 1, "a node was born from the drag");
        let expected_y = ((mid[1] + 4.0) * 2.0).round() / 2.0; // drag end, half-snapped
        assert_eq!(net.junctions[0].pos[1], expected_y);
        assert_eq!(net.segments.len(), 2);
        // Still one connected net with the same logic.
        assert_eq!(net.sinks.len(), 1);
    }

    #[test]
    fn box_select_includes_wires() {
        let mut d = Driver::new();
        wired_pair(&mut d);
        d.drag([-2.0, -1.0], [14.0, 9.0]);
        assert_eq!(d.app.selection.components.len(), 2);
        assert_eq!(d.app.selection.segments.len(), 1, "the wire came along");
        d.app.handle_key(KeyInput::Delete);
        assert!(d.app.editor.project.components.is_empty());
        assert!(d.app.editor.project.wire_nets.is_empty());
    }

    #[test]
    fn output_pin_wire_selects_and_unwires() {
        let mut d = Driver::new();
        d.app.action_new_abstraction();
        d.type_text("box");
        d.app.handle_key(KeyInput::Enter);
        d.app.action_add_pin(PortKind::Output);
        d.type_text("q");
        d.app.handle_key(KeyInput::Enter);
        d.place("nand", [2.0, 1.0]);
        d.app.camera.offset = [300.0, 250.0];
        let abstractions = &d.app.editor.project.abstractions;
        let pin_pos = diycpu::layout::interface_pins(abstractions, &abstractions[0])
            .into_iter()
            .find(|p| p.name == "q")
            .unwrap()
            .pos;
        d.drag(port_at(&d.app, "nand_1", "out"), pin_pos);
        assert_eq!(
            d.app.editor.project.abstractions[0].outputs.get("q"),
            Some(&"nand_1".to_string())
        );

        // Click the derived wire midway and delete it.
        let from = port_at(&d.app, "nand_1", "out");
        let mid = [(from[0] + pin_pos[0]) / 2.0, (from[1] + pin_pos[1]) / 2.0];
        d.app.handle_key(KeyInput::Escape);
        d.click(mid);
        assert_eq!(d.app.selection.pin_wires, vec!["q".to_string()]);
        d.move_to([0.0, -6.0]);
        d.app.handle_key(KeyInput::Delete);
        assert_eq!(
            d.app.editor.project.abstractions[0].outputs.get("q"),
            Some(&String::new())
        );
    }
}
