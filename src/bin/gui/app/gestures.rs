//! The gesture state machine: every mouse and keyboard event lands
//! here, collects intent, and commits through an Editor op exactly once.
//! Dropping a gesture is always safe — nothing here touches the document
//! until the commit.


use diycpu::editor::{EditError, SheetRef, WirePoint};
use diycpu::layout::{self, Hit, PortKind, Rect};
use diycpu::project::PortRef;

use crate::theme;

use super::*;


impl App {
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
    pub(super) fn port_wire_kind(&self, port: &PortRef) -> Option<PortKind> {
        if port.component_id.starts_with(diycpu::convert::INPUT_PREFIX) {
            return Some(PortKind::Output);
        }
        if port.component_id.starts_with(layout::OUTPUT_PREFIX) {
            return Some(PortKind::Input);
        }
        let (components, _) = self.editor.sheet_data();
        let comp = components.iter().find(|c| c.id == port.component_id)?;
        layout::component_port_info(&self.editor.project.abstractions, comp)?
            .into_iter()
            .find(|p| p.name == port.port_name)
            .map(|p| p.kind)
    }

    /// Signal or bundle, for any port on the current sheet.
    pub fn port_lane(&self, port: &PortRef) -> Option<layout::WireKind> {
        let (components, _) = self.editor.sheet_data();
        layout::wire_kind_any(
            &self.editor.project.abstractions,
            components,
            self.interface(),
            port,
        )
    }

    /// The lane (signal / bundle) the wire being drawn from `anchor` is in.
    pub(super) fn anchor_lane(&self, anchor: &WirePoint) -> layout::WireKind {
        match anchor {
            WirePoint::Port(p) => self.port_lane(p).unwrap_or(layout::WireKind::Signal),
            WirePoint::Junction { net_id, .. } => {
                let (_, wire_nets) = self.editor.sheet_data();
                let w = wire_nets
                    .iter()
                    .find(|n| &n.id == net_id)
                    .filter(|n| n.bundle)
                    .and_then(|n| {
                        n.source
                            .as_ref()
                            .and_then(|s| self.port_lane(s))
                            .or(Some(layout::WireKind::Bundle(0)))
                    });
                w.unwrap_or(layout::WireKind::Signal)
            }
            WirePoint::New(_) => layout::WireKind::Signal,
        }
    }

    /// Does the wire being drawn already carry a driving source?
    pub(super) fn anchor_drives(&self, anchor: &WirePoint) -> bool {
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

    /// What would make a pin dropped at `pos` look falsely connected —
    /// None when the spot is clear (or only touches notes/images/itself).
    pub(super) fn pin_drop_conflict(&self, name: &str, pos: [f32; 2]) -> Option<String> {
        let (components, wire_nets) = self.editor.sheet_data();
        let hit = layout::hit_test(
            &self.editor.project.abstractions,
            components,
            wire_nets,
            &[],
            &[],
            self.interface(),
            pos,
        );
        let own_input = format!("{}{name}", diycpu::convert::INPUT_PREFIX);
        let own_output = format!("{}{name}", layout::OUTPUT_PREFIX);
        match hit {
            Hit::Empty | Hit::Note { .. } | Hit::Image { .. } => None,
            Hit::Pin { name: n, .. } if n == name => None,
            Hit::Port { component_id, .. }
                if component_id == own_input || component_id == own_output =>
            {
                None
            }
            Hit::Component { .. } => {
                Some("A pin can't sit on a component — drop it in the open".into())
            }
            _ => Some(
                "That would only look connected — drop the pin in the open, then wire it"
                    .into(),
            ),
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
                // Bundles and single wires never join.
                let same_lane = matches!(
                    (self.anchor_lane(anchor), self.port_lane(&target)),
                    (layout::WireKind::Signal, Some(layout::WireKind::Signal))
                        | (layout::WireKind::Bundle(_), Some(layout::WireKind::Bundle(_)))
                );
                if !same_lane {
                    return WireTarget::Incompatible;
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
                // Merging two driven nets is invalid (unless same source),
                // and bundles never join signal wires.
                let (_, wire_nets) = self.editor.sheet_data();
                let target_bundle = wire_nets
                    .iter()
                    .find(|n| &n.id == net_id)
                    .map(|n| n.bundle)
                    .unwrap_or(false);
                let anchor_bundle =
                    matches!(self.anchor_lane(anchor), layout::WireKind::Bundle(_));
                if target_bundle != anchor_bundle {
                    return WireTarget::Incompatible;
                }
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

    pub fn mouse_moved(&mut self, pos: [f32; 2]) {
        self.cursor = pos;
        // Modal layers freeze the canvas beneath them.
        if self.any_modal() {
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
                    let (ids, note_ids, image_ids) =
                        if self.selection.components.contains(id) {
                            (
                                self.selection.components.clone(),
                                self.selection.notes.clone(),
                                self.selection.images.clone(),
                            )
                        } else {
                            let ids = vec![id.clone()];
                            self.selection.clear();
                            self.selection.components = ids.clone();
                            (ids, vec![], vec![])
                        };
                    let press_grid = self.camera.screen_to_grid(*press_screen);
                    self.gesture = Gesture::DraggingComponents {
                        ids,
                        note_ids,
                        image_ids,
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
                    let (ids, note_ids, image_ids) = if self.selection.notes.contains(id) {
                        (
                            self.selection.components.clone(),
                            self.selection.notes.clone(),
                            self.selection.images.clone(),
                        )
                    } else {
                        let note_ids = vec![id.clone()];
                        self.selection.clear();
                        self.selection.notes = note_ids.clone();
                        (vec![], note_ids, vec![])
                    };
                    let press_grid = self.camera.screen_to_grid(*press_screen);
                    self.gesture = Gesture::DraggingComponents {
                        ids,
                        note_ids,
                        image_ids,
                        press_grid,
                        delta: [0, 0],
                        copying,
                    };
                }
            }
            Gesture::PressedImage {
                id,
                press_screen,
                ctrl,
            } => {
                if dist(pos, *press_screen) > DRAG_THRESHOLD_PX {
                    let copying = *ctrl;
                    let (ids, note_ids, image_ids) = if self.selection.images.contains(id) {
                        (
                            self.selection.components.clone(),
                            self.selection.notes.clone(),
                            self.selection.images.clone(),
                        )
                    } else {
                        let image_ids = vec![id.clone()];
                        self.selection.clear();
                        self.selection.images = image_ids.clone();
                        (vec![], vec![], image_ids)
                    };
                    let press_grid = self.camera.screen_to_grid(*press_screen);
                    self.gesture = Gesture::DraggingComponents {
                        ids,
                        note_ids,
                        image_ids,
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
        if self.image_dialog.is_some() || self.display_dialog.is_some() {
            return; // the shell routes these to the dialog handlers
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
            Gesture::PlacingImage { source, size } => {
                let (source, size) = (source.clone(), *size);
                let g = self.cursor_grid();
                let pos = [
                    ((g[0] - size[0] / 2.0) * 2.0).round() / 2.0,
                    ((g[1] - size[1] / 2.0) * 2.0).round() / 2.0,
                ];
                // The file is copied into the project only now, on commit —
                // cancelling a placement never leaves stray media behind.
                match self.import_media(&source) {
                    Ok(rel) => match self.editor.place_image(pos, &rel, size) {
                        Ok(id) => {
                            self.selection.clear();
                            self.selection.images = vec![id];
                            self.info("Image placed — hover it and click the cog to adjust");
                        }
                        Err(e) => self.error(e.to_string()),
                    },
                    Err(e) => self.error(e),
                }
                self.gesture = Gesture::Idle;
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
                Hit::Image { image_id } => {
                    self.selection.clear();
                    self.selection.images = vec![image_id];
                }
                _ => self.selection.clear(),
            }
            return;
        }

        // The hover cogwheel wins over whatever is underneath it.
        if let Some((rect, action)) = self.cog() {
            if rect.contains(self.cursor) {
                self.perform_cog(action);
                return;
            }
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
            Hit::Image { image_id } => {
                if !self.ctrl && !self.selection.images.contains(&image_id) {
                    self.selection.clear();
                    self.selection.images = vec![image_id.clone()];
                }
                self.gesture = Gesture::PressedImage {
                    id: image_id,
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
    pub(super) fn wire_click(&mut self) {
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
            gesture @ (Gesture::Placing { .. } | Gesture::PlacingImage { .. }) => {
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
            Gesture::PressedImage { id, ctrl, .. } => {
                if ctrl {
                    if let Some(i) = self.selection.images.iter().position(|n| n == &id) {
                        self.selection.images.remove(i);
                    } else {
                        self.selection.images.push(id);
                    }
                    self.selection.pin = None;
                } else {
                    self.selection.clear();
                    self.selection.images = vec![id];
                }
            }
            Gesture::DraggingComponents {
                ids,
                note_ids,
                image_ids,
                delta,
                copying,
                ..
            } => {
                if copying {
                    if delta != [0, 0] {
                        let fragment = self.editor.fragment(&ids, &note_ids, &image_ids);
                        match self.editor.paste_fragment(&fragment, delta, false) {
                            Ok((new_ids, new_notes, new_images)) => {
                                self.selection.clear();
                                self.selection.components = new_ids;
                                self.selection.notes = new_notes;
                                self.selection.images = new_images;
                                self.info("Copied");
                            }
                            Err(e) => self.error(e.to_string()),
                        }
                    }
                } else {
                    match self.editor.move_items(&ids, &note_ids, &image_ids, delta) {
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
                let images: Vec<String> = self
                    .editor
                    .sheet_images()
                    .iter()
                    .filter(|i| layout::image_bounds(i).intersects(&band))
                    .map(|i| i.id.clone())
                    .collect();
                // Wire segments come along when the band touches any part
                // of them — the same feel components have.
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
                            if layout::segment_intersects_rect(a, b, &band) {
                                segments.push((net.id.clone(), i));
                            }
                        }
                    }
                }
                // Derived output-pin wires sweep up the same way.
                let mut pin_wires: Vec<String> = Vec::new();
                if let Some(def) = self.interface() {
                    let pins = layout::interface_pins(abstractions, def);
                    for (name, target) in &def.outputs {
                        if target.is_empty() {
                            continue;
                        }
                        let Some(pin) = pins
                            .iter()
                            .find(|p| p.io == PortKind::Output && &p.name == name)
                        else {
                            continue;
                        };
                        let port = layout::output_target_port(target);
                        let Some((from, _)) = layout::port_geom_any(
                            abstractions,
                            sheet_components,
                            self.interface(),
                            &port,
                        ) else {
                            continue;
                        };
                        if layout::segment_intersects_rect(from, pin.pos, &band) {
                            pin_wires.push(name.clone());
                        }
                    }
                }
                self.selection.clear();
                self.selection.components = components;
                self.selection.notes = notes;
                self.selection.images = images;
                self.selection.segments = segments;
                self.selection.pin_wires = pin_wires;
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
                // A pin sitting on a port, node or wire would LOOK connected
                // without being connected — refuse rather than lie.
                if let Some(why) = self.pin_drop_conflict(&name, pos) {
                    self.error(why);
                } else {
                    match self.editor.set_pin_position(&name, pos) {
                        Ok(()) => {}
                        Err(e) => self.error(e.to_string()),
                    }
                }
            }
        }
    }

    pub fn middle_or_right_pressed(&mut self) {
        if self.any_modal() {
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
        if self.any_modal() {
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
        if let Hit::Image { image_id } = self.hover.clone() {
            self.action_open_image_dialog(&image_id);
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
            let image_ids = self.selection.images.clone();
            let segments = self.selection.segments.clone();
            let pin_wires = self.selection.pin_wires.clone();
            let n =
                ids.len() + note_ids.len() + image_ids.len() + segments.len() + pin_wires.len();
            match self
                .editor
                .delete_mixed(&ids, &note_ids, &image_ids, &segments, &pin_wires)
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
        if self.image_dialog.is_some() {
            if matches!(key, KeyInput::Escape | KeyInput::Enter) {
                self.image_dialog = None;
            }
            return;
        }
        if self.display_dialog.is_some() {
            if matches!(key, KeyInput::Escape | KeyInput::Enter) {
                self.display_dialog = None;
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

    pub(super) fn key_char(&mut self, ch: &str) {
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
        // Placement gestures carry no pointer state and survive — the file
        // picker itself steals focus right before image placement starts.
        if !matches!(
            self.gesture,
            Gesture::Placing { .. } | Gesture::PlacingImage { .. }
        ) {
            self.gesture = Gesture::Idle;
        }
        self.ctrl = false;
        self.shift = false;
    }
}

/// Pixels the cursor must travel before a press becomes a drag.
pub(crate) const DRAG_THRESHOLD_PX: f32 = 5.0;

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

pub(crate) fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

pub(crate) fn snap_half(g: [f32; 2]) -> [f32; 2] {
    [(g[0] * 2.0).round() / 2.0, (g[1] * 2.0).round() / 2.0]
}

pub(crate) fn rect_between(a: [f32; 2], b: [f32; 2]) -> Rect {
    Rect {
        x: a[0].min(b[0]),
        y: a[1].min(b[1]),
        w: (a[0] - b[0]).abs(),
        h: (a[1] - b[1]).abs(),
    }
}
