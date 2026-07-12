//! Application state and interaction logic. No drawing happens here; the
//! renderer reads this state. No document mutation happens here either —
//! gestures collect intent and commit through `Editor` operations exactly
//! once, so cancelling a gesture (Esc, right-click, focus loss) is always
//! just dropping it.

use std::path::PathBuf;

use diycpu::editor::{EditError, Editor, SheetRef, WirePoint};
use diycpu::layout::{self, Hit, PortKind, Rect};
use diycpu::project::{PortRef, Project};

use crate::camera::Camera;
use crate::sim::SimSession;
use crate::theme;

/// Pixels the cursor must travel before a press becomes a drag.
const DRAG_THRESHOLD_PX: f32 = 5.0;

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
    /// Left button down on a component; not yet decided click vs drag.
    PressedComponent {
        id: String,
        press_screen: [f32; 2],
    },
    DraggingComponents {
        ids: Vec<String>,
        press_grid: [f32; 2],
        delta: [i32; 2],
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
}

#[derive(Default)]
pub struct Selection {
    pub components: Vec<String>,
    pub net: Option<String>,
    /// A selected interface pin (kind, name) inside an abstraction sheet.
    pub pin: Option<(PortKind, String)>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.components.clear();
        self.net = None;
        self.pin = None;
    }
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.net.is_none() && self.pin.is_none()
    }
}

/// What a text prompt, once confirmed, will do.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromptAction {
    NewAbstraction,
    AddInputPin,
    AddOutputPin,
}

pub struct Prompt {
    pub title: String,
    pub value: String,
    pub action: PromptAction,
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
    /// Run mode advances the clock on a timer.
    pub playing: bool,
    /// Set when Ctrl+N is pressed with unsaved changes; a second press confirms.
    confirm_new: bool,
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
            playing: false,
            confirm_new: false,
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
                self.interface(),
                g,
            )
        } else {
            Hit::Empty
        };
    }

    /// The palette entries: primitives, then abstraction types.
    pub fn palette_items(&self) -> Vec<String> {
        let mut items = vec!["clk".to_string(), "nand".to_string(), "dff".to_string()];
        items.extend(self.editor.project.abstractions.iter().map(|a| a.name.clone()));
        items
    }

    /// Snapped top-left placement position so the ghost centers on the cursor.
    pub fn placement_pos(&self, type_name: &str) -> [i32; 2] {
        let size = layout::type_size(&self.editor.project.abstractions, type_name)
            .unwrap_or([3.0, 2.0]);
        let g = self.cursor_grid();
        [
            (g[0] - size[0] / 2.0).round() as i32,
            (g[1] - size[1] / 2.0).round() as i32,
        ]
    }

    /// Would placing/moving these footprints collide? Used by ghost previews;
    /// the editor re-validates authoritatively on commit.
    pub fn placement_is_valid(&self, type_name: &str) -> bool {
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

    pub fn drag_is_valid(&self, ids: &[String], delta: [i32; 2]) -> bool {
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
            .filter(|c| !ids.contains(&c.id))
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

    pub fn save(&mut self) {
        let path = self
            .file_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(format!("{}.json", self.editor.project.name)));
        match serde_json::to_string_pretty(&self.editor.project)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&path, json + "\n").map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.editor.dirty = false;
                self.file_path = Some(path.clone());
                self.info(format!("Saved {}", path.display()));
            }
            Err(e) => self.error(format!("Save failed: {e}")),
        }
    }

    fn new_project(&mut self) {
        if self.editor.dirty && !self.confirm_new {
            self.confirm_new = true;
            self.error("Unsaved changes — press Ctrl+N again to discard them");
            return;
        }
        self.confirm_new = false;
        self.editor = Editor::empty();
        self.file_path = None;
        self.selection.clear();
        self.gesture = Gesture::Idle;
        self.mode = Mode::Edit;
        self.pending_fit = true;
        self.info("New circuit");
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
                self.gesture = Gesture::Idle;
                // Simulation always runs the whole design from the top.
                if self.editor.sheet != SheetRef::Top {
                    let _ = self.editor.open_sheet(SheetRef::Top);
                    self.selection.clear();
                    self.pending_fit = true;
                }
                match SimSession::new(&self.editor.project) {
                    Ok(session) => {
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
        // Interface pins live on rails beside the content — include them.
        if let Some(def) = self.interface() {
            for pin in layout::interface_pins(abstractions, def) {
                let r = pin.plate;
                bounds = Some(match bounds {
                    None => r,
                    Some(b) => {
                        let x = b.x.min(r.x);
                        let y = b.y.min(r.y);
                        let x2 = (b.x + b.w).max(r.x + r.w);
                        let y2 = (b.y + b.h).max(r.y + r.h);
                        Rect { x, y, w: x2 - x, h: y2 - y }
                    }
                });
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
        let previous = self.cursor;
        self.cursor = pos;

        match &mut self.gesture {
            Gesture::Panning { last } => {
                let dx = pos[0] - last[0];
                let dy = pos[1] - last[1];
                *last = pos;
                self.camera.pan(dx, dy);
            }
            Gesture::PressedComponent { id, press_screen } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    // Promote to drag: the pressed component leads; if it's
                    // part of the selection the whole selection comes along.
                    let ids = if self.selection.components.contains(id) {
                        self.selection.components.clone()
                    } else {
                        let ids = vec![id.clone()];
                        self.selection.components = ids.clone();
                        self.selection.net = None;
                        ids
                    };
                    let press_grid = self.camera.screen_to_grid(*press_screen);
                    self.gesture = Gesture::DraggingComponents {
                        ids,
                        press_grid,
                        delta: [0, 0],
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
            _ => {}
        }

        let _ = previous;
        self.recompute_hover();
    }

    pub fn left_pressed(&mut self) {
        self.confirm_new = false;

        // A click outside an open prompt dismisses it.
        if self.prompt.is_some() {
            self.prompt = None;
            self.info("Cancelled");
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
                match self.editor.place_component(&type_name, pos) {
                    Ok(id) => {
                        self.selection.components = vec![id.clone()];
                        self.selection.net = None;
                        self.info(format!("Placed {id}"));
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
                    self.selection.components = vec![component_id];
                    self.selection.net = None;
                }
                Hit::Wire { net_id, .. } => {
                    self.selection.clear();
                    self.selection.net = Some(net_id);
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
                if self.ctrl {
                    // Toggle in/out of the multi-selection immediately.
                    if let Some(i) = self
                        .selection
                        .components
                        .iter()
                        .position(|c| c == &component_id)
                    {
                        self.selection.components.remove(i);
                    } else {
                        self.selection.components.push(component_id);
                    }
                    self.selection.net = None;
                } else {
                    if !self.selection.components.contains(&component_id) {
                        self.selection.components = vec![component_id.clone()];
                        self.selection.net = None;
                    }
                    self.gesture = Gesture::PressedComponent {
                        id: component_id,
                        press_screen: self.cursor,
                    };
                }
            }
            Hit::Pin { name, kind } => {
                self.selection.clear();
                self.selection.pin = Some((kind, name.clone()));
                self.info(format!("Pin \"{name}\" selected — Delete removes it"));
            }
            Hit::Wire { net_id, .. } => {
                self.selection.clear();
                self.selection.net = Some(net_id);
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
            Gesture::PressedComponent { id, .. } => {
                // A click (no drag): collapse the selection to this component.
                self.selection.components = vec![id];
                self.selection.net = None;
            }
            Gesture::DraggingComponents { ids, delta, .. } => {
                match self.editor.move_components(&ids, delta) {
                    Ok(()) => {}
                    Err(e) => self.error(e.to_string()),
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
                self.selection.components = components
                    .iter()
                    .filter(|c| layout::footprint(abstractions, c).intersects(&band))
                    .map(|c| c.id.clone())
                    .collect();
                self.selection.net = None;
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
        }
    }

    pub fn middle_or_right_pressed(&mut self) -> PanOrCancel {
        // An active constructive gesture? Right/middle click ends it.
        match self.gesture {
            Gesture::Placing { .. } => {
                self.gesture = Gesture::Idle;
                self.info("Cancelled");
                PanOrCancel::Cancelled
            }
            Gesture::DrawingWire { .. } => {
                // Everything placed so far is already committed; this just
                // puts the pencil down.
                self.gesture = Gesture::Idle;
                self.info("Stopped wiring — click a node to continue from it");
                PanOrCancel::Cancelled
            }
            _ => {
                self.gesture = Gesture::Panning { last: self.cursor };
                PanOrCancel::Panning
            }
        }
    }

    pub fn pan_released(&mut self) {
        if matches!(self.gesture, Gesture::Panning { .. }) {
            self.gesture = Gesture::Idle;
        }
    }

    pub fn scrolled(&mut self, amount: f32) {
        if !self.canvas_rect().contains(self.cursor) {
            return;
        }
        let factor = (1.0015f32).powf(amount);
        self.camera.zoom_at(self.cursor, factor);
        self.recompute_hover();
    }

    pub fn double_clicked(&mut self) {
        if !self.canvas_rect().contains(self.cursor) || self.is_running() {
            return;
        }
        if let Hit::Component { component_id } = self.hover.clone() {
            let (components, _) = self.editor.sheet_data();
            let Some(comp) = components.iter().find(|c| c.id == component_id) else {
                return;
            };
            if comp.component_type == "dff" {
                match self.editor.toggle_dff_init(&component_id) {
                    Ok(v) => self.info(format!(
                        "{component_id} now starts at {}",
                        if v { "1" } else { "0" }
                    )),
                    Err(e) => self.error(e.to_string()),
                }
                return;
            }
            // Double-click an abstraction instance to open its sheet.
            let type_name = comp.component_type.clone();
            if let Some(i) = self
                .editor
                .project
                .abstractions
                .iter()
                .position(|a| a.name == type_name)
            {
                match self.editor.open_sheet(SheetRef::Abstraction(i)) {
                    Ok(()) => {
                        self.selection.clear();
                        self.gesture = Gesture::Idle;
                        self.pending_fit = true;
                        self.info(format!(
                            "Editing \"{type_name}\" — every instance updates; Esc goes back"
                        ));
                    }
                    Err(e) => self.error(e.to_string()),
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
        } else if self.editor.sheet != SheetRef::Top {
            let _ = self.editor.open_sheet(SheetRef::Top);
            self.selection.clear();
            self.pending_fit = true;
            self.info("Back to the main circuit");
        }
    }

    pub fn delete_selection(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        // A hovered wire node takes priority: point at it and press Delete.
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
        if let Some((kind, name)) = self.selection.pin.clone() {
            match self.editor.remove_interface_pin(kind, &name) {
                Ok(()) => {
                    self.selection.pin = None;
                    self.info(format!("Removed pin \"{name}\""));
                }
                Err(e) => self.error(e.to_string()),
            }
        } else if let Some(net_id) = self.selection.net.clone() {
            match self.editor.delete_net(&net_id) {
                Ok(()) => {
                    self.selection.net = None;
                    self.info("Deleted wire");
                }
                Err(e) => self.error(e.to_string()),
            }
        } else if !self.selection.components.is_empty() {
            let ids = self.selection.components.clone();
            let n = ids.len();
            match self.editor.delete_components(&ids) {
                Ok(()) => {
                    self.selection.clear();
                    self.info(if n == 1 {
                        "Deleted 1 component".to_string()
                    } else {
                        format!("Deleted {n} components")
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
        match self.editor.undo() {
            Ok(()) => self.info("Undone"),
            Err(e) => self.info(e.to_string()),
        }
    }

    pub fn redo(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        self.gesture = Gesture::Idle;
        self.selection.clear();
        match self.editor.redo() {
            Ok(()) => self.info("Redone"),
            Err(e) => self.info(e.to_string()),
        }
    }

    pub fn duplicate(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        if self.selection.components.is_empty() {
            self.info("Nothing selected to duplicate");
            return;
        }
        let ids = self.selection.components.clone();
        match self.editor.duplicate_components(&ids) {
            Ok(new_ids) => {
                self.info(format!("Duplicated {} component(s)", new_ids.len()));
                self.selection.components = new_ids;
                self.selection.net = None;
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
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        prompt.value.push(c);
                    }
                }
            }
            _ => {}
        }
    }

    fn commit_prompt(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        let name = prompt.value.trim().to_string();
        if name.is_empty() {
            self.error("A name is required");
            self.prompt = Some(prompt);
            return;
        }
        match prompt.action {
            PromptAction::NewAbstraction => match self.editor.create_abstraction(&name) {
                Ok(i) => {
                    let _ = self.editor.open_sheet(SheetRef::Abstraction(i));
                    self.selection.clear();
                    self.pending_fit = true;
                    self.info(format!(
                        "Created \"{name}\" — add pins and parts, then Esc to go back"
                    ));
                }
                Err(e) => {
                    self.error(e.to_string());
                    self.prompt = Some(prompt);
                }
            },
            PromptAction::AddInputPin | PromptAction::AddOutputPin => {
                let kind = if prompt.action == PromptAction::AddInputPin {
                    PortKind::Input
                } else {
                    PortKind::Output
                };
                match self.editor.add_interface_pin(kind, &name) {
                    Ok(()) => self.info(format!("Added pin \"{name}\"")),
                    Err(e) => {
                        self.error(e.to_string());
                        self.prompt = Some(prompt);
                    }
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
        });
    }

    pub fn action_back_to_top(&mut self) {
        if self.editor.sheet != SheetRef::Top {
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
            ("s", true, _) => self.save(),
            ("n", true, _) => {
                if self.require_edit_mode() {
                    self.new_project()
                }
            }
            ("d", true, _) => self.duplicate(),
            ("p", false, _) => self.action_play_toggle(),
            ("f", false, _) => {
                self.fit_view();
                self.info("View fitted to circuit");
            }
            ("0", true, _) => {
                self.camera.zoom = 1.0;
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

pub enum PanOrCancel {
    Panning,
    Cancelled,
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
    struct Driver {
        app: App,
    }

    impl Driver {
        fn new() -> Self {
            let mut app = App::new(None);
            app.window_size = [1280.0, 720.0];
            app.pending_fit = false;
            // Park the origin comfortably inside the canvas.
            app.camera.offset = [400.0, 200.0];
            Driver { app }
        }

        fn move_to(&mut self, g: [f32; 2]) {
            let s = self.app.camera.grid_to_screen(g);
            self.app.mouse_moved(s);
        }

        fn click(&mut self, g: [f32; 2]) {
            self.move_to(g);
            self.app.left_pressed();
            self.app.left_released();
            // The shell recomputes hover every frame; mimic that.
            self.app.recompute_hover();
        }

        fn drag(&mut self, from: [f32; 2], to: [f32; 2]) {
            self.move_to(from);
            self.app.left_pressed();
            // Two moves so the drag threshold is crossed mid-gesture.
            self.move_to([(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0]);
            self.move_to(to);
            self.app.left_released();
            self.app.recompute_hover();
        }

        fn place(&mut self, type_name: &str, center: [f32; 2]) {
            self.app.palette_clicked(type_name);
            self.click(center);
        }

        fn type_text(&mut self, s: &str) {
            for c in s.chars() {
                self.app.handle_key(KeyInput::Char(&c.to_string()));
            }
        }
    }

    fn port_at(app: &App, comp: &str, port: &str) -> [f32; 2] {
        let (components, _) = app.editor.sheet_data();
        let c = components.iter().find(|c| c.id == comp).unwrap();
        diycpu::layout::port_pos(&app.editor.project.abstractions, c, port)
            .unwrap()
            .pos
    }

    fn center_of(app: &App, comp: &str) -> [f32; 2] {
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
            session.port_value(
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
