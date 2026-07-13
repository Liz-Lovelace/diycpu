//! The editing layer: owns the `Project` document and applies validated,
//! atomic, undoable operations to it.
//!
//! Every mutation goes through [`Editor::mutate`], which snapshots the
//! document first and rolls back completely if the operation reports an
//! error — an operation can therefore never leave the document half-mutated.
//! One successful operation = exactly one undo step.
//!
//! Operations work on a *sheet*: either the top-level circuit or the inside
//! of one abstraction. Both have the same shape (components + wire nets),
//! so the same operations serve both.

use std::collections::HashMap;
use std::fmt;

use crate::layout::{self, PortKind, Rect};
use crate::project::{
    Endpoint, Junction, PortRef, Project, ProjectComponent, WireNet,
};

/// Which circuit sheet operations apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetRef {
    Top,
    /// Index into `project.abstractions`.
    Abstraction(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EditError {
    UnknownComponentType(String),
    UnknownComponent(String),
    UnknownPort(String, String),
    Overlap,
    RecursiveAbstraction(String),
    NotAnInputAndOutput,
    NotADff(String),
    BadSheet,
    NothingToUndo,
    NothingToRedo,
    InvalidName(String),
    NameTaken(String),
    NotAnAbstractionSheet,
    PassthroughNotAllowed,
    UnknownJunction(String),
    TwoSources(String),
    InputDoublyWired(String),
    /// The requested wire already exists / connects a point to itself —
    /// harmless, callers usually ignore it silently.
    NoopWire,
    OutputPinNeedsSource,
    AbstractionInUse(usize),
    UnknownNote(String),
    UnknownNet(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponentType(t) => write!(f, "Unknown component type \"{t}\""),
            Self::UnknownComponent(id) => write!(f, "No component \"{id}\""),
            Self::UnknownPort(c, p) => write!(f, "Component \"{c}\" has no port \"{p}\""),
            Self::Overlap => write!(f, "Components can't overlap"),
            Self::RecursiveAbstraction(name) => {
                write!(f, "Placing \"{name}\" here would nest it inside itself")
            }
            Self::NotAnInputAndOutput => {
                write!(f, "A wire connects an output to an input")
            }
            Self::NotADff(id) => write!(f, "\"{id}\" is not a flip-flop"),
            Self::BadSheet => write!(f, "This sheet no longer exists"),
            Self::NothingToUndo => write!(f, "Nothing to undo"),
            Self::NothingToRedo => write!(f, "Nothing to redo"),
            Self::InvalidName(n) => {
                write!(f, "\"{n}\" — names use letters, digits, _ and -")
            }
            Self::NameTaken(n) => write!(f, "The name \"{n}\" is already used"),
            Self::NotAnAbstractionSheet => {
                write!(f, "Pins can only be edited inside an abstraction")
            }
            Self::PassthroughNotAllowed => {
                write!(f, "Route inputs to outputs through at least one component")
            }
            Self::UnknownJunction(id) => write!(f, "That wire node ({id}) no longer exists"),
            Self::TwoSources(existing) => {
                write!(f, "This wire is already driven by {existing}")
            }
            Self::InputDoublyWired(port) => {
                write!(f, "{port} is already connected to this wire")
            }
            Self::NoopWire => write!(f, "Those points are already connected"),
            Self::OutputPinNeedsSource => {
                write!(f, "This wire has no output driving it yet — connect one first")
            }
            Self::AbstractionInUse(n) => write!(
                f,
                "This abstraction is placed {n} time(s) — delete those instances first"
            ),
            Self::UnknownNote(id) => write!(f, "That note ({id}) no longer exists"),
            Self::UnknownNet(id) => write!(f, "That wire ({id}) no longer exists"),
        }
    }
}

/// Names for abstractions and pins: they become engine identifiers, so no
/// dots, colons or whitespace.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

const UNDO_LIMIT: usize = 200;

pub struct Editor {
    pub project: Project,
    /// The sheet currently open in the workbench.
    pub sheet: SheetRef,
    undo_stack: Vec<(Project, SheetRef)>,
    redo_stack: Vec<(Project, SheetRef)>,
    pub dirty: bool,
}

impl Editor {
    pub fn new(project: Project) -> Self {
        Editor {
            project,
            sheet: SheetRef::Top,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            dirty: false,
        }
    }

    pub fn empty() -> Self {
        Self::new(Project {
            name: "untitled".into(),
            steps: 16,
            abstractions: vec![],
            components: vec![],
            wire_nets: vec![],
            notes: vec![],
        })
    }

    // ── Sheet access ─────────────────────────────────────────────────────

    /// The components and wire nets of the current sheet.
    pub fn sheet_data(&self) -> (&[ProjectComponent], &[WireNet]) {
        sheet_of(&self.project, self.sheet)
            .expect("current sheet is always kept valid")
    }

    /// The annotations on the current sheet.
    pub fn sheet_notes(&self) -> &[crate::project::Note] {
        sheet_notes(&self.project, self.sheet)
            .expect("current sheet is always kept valid")
    }

    pub fn open_sheet(&mut self, sheet: SheetRef) -> Result<(), EditError> {
        if sheet_of(&self.project, sheet).is_none() {
            return Err(EditError::BadSheet);
        }
        self.sheet = sheet;
        Ok(())
    }

    // ── Undo machinery ───────────────────────────────────────────────────

    /// Run one atomic operation. On success it becomes one undo step; on
    /// error the document is exactly as before the call.
    fn mutate<R>(
        &mut self,
        op: impl FnOnce(&mut Project, SheetRef) -> Result<R, EditError>,
    ) -> Result<R, EditError> {
        let before = self.project.clone();
        match op(&mut self.project, self.sheet) {
            Ok(result) => {
                self.undo_stack.push((before, self.sheet));
                if self.undo_stack.len() > UNDO_LIMIT {
                    self.undo_stack.remove(0);
                }
                self.redo_stack.clear();
                self.dirty = true;
                Ok(result)
            }
            Err(e) => {
                self.project = before;
                Err(e)
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo(&mut self) -> Result<(), EditError> {
        let (project, sheet) = self.undo_stack.pop().ok_or(EditError::NothingToUndo)?;
        self.redo_stack
            .push((std::mem::replace(&mut self.project, project), self.sheet));
        self.sheet = sheet;
        self.dirty = true;
        Ok(())
    }

    pub fn redo(&mut self) -> Result<(), EditError> {
        let (project, sheet) = self.redo_stack.pop().ok_or(EditError::NothingToRedo)?;
        self.undo_stack
            .push((std::mem::replace(&mut self.project, project), self.sheet));
        self.sheet = sheet;
        self.dirty = true;
        Ok(())
    }

    // ── Operations ───────────────────────────────────────────────────────

    /// Place a new component (primitive or abstraction instance) with its
    /// top-left corner at integer grid position `pos`.
    pub fn place_component(
        &mut self,
        type_name: &str,
        pos: [i32; 2],
    ) -> Result<String, EditError> {
        let type_name = type_name.to_string();
        self.mutate(move |project, sheet| {
            let size = layout::type_size(&project.abstractions, &type_name)
                .ok_or_else(|| EditError::UnknownComponentType(type_name.clone()))?;

            // An abstraction may not (transitively) contain itself.
            if let SheetRef::Abstraction(i) = sheet {
                let host = project
                    .abstractions
                    .get(i)
                    .ok_or(EditError::BadSheet)?
                    .name
                    .clone();
                if abstraction_uses(&project.abstractions, &type_name, &host) {
                    return Err(EditError::RecursiveAbstraction(type_name.clone()));
                }
            }

            let rect = Rect {
                x: pos[0] as f32,
                y: pos[1] as f32,
                w: size[0],
                h: size[1],
            };
            let abstractions = project.abstractions.clone();
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for existing in components.iter() {
                if layout::footprint(&abstractions, existing).intersects(&rect) {
                    return Err(EditError::Overlap);
                }
            }

            let id = generate_id(components, &type_name);
            components.push(ProjectComponent {
                id: id.clone(),
                component_type: type_name.clone(),
                init: (type_name == "dff" || type_name == "const").then_some(false),
                pos: [pos[0] as f32, pos[1] as f32],
            });
            Ok(id)
        })
    }

    /// Move a set of components by a whole number of grid cells. Rejected if
    /// any moved footprint would overlap a component outside the set.
    pub fn move_components(
        &mut self,
        ids: &[String],
        delta: [i32; 2],
    ) -> Result<(), EditError> {
        self.move_items(ids, &[], delta)
    }

    /// Move components and notes together by a whole number of grid cells,
    /// as one undo step. Only components have overlap rules.
    pub fn move_items(
        &mut self,
        component_ids: &[String],
        note_ids: &[String],
        delta: [i32; 2],
    ) -> Result<(), EditError> {
        if delta == [0, 0] {
            return Ok(()); // not an edit; don't spend an undo step
        }
        let ids = component_ids.to_vec();
        let note_ids = note_ids.to_vec();
        self.mutate(move |project, sheet| {
            let abstractions = project.abstractions.clone();
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;

            let mut moved_rects = Vec::new();
            for id in &ids {
                let comp = components
                    .iter()
                    .find(|c| &c.id == id)
                    .ok_or_else(|| EditError::UnknownComponent(id.clone()))?;
                let mut rect = layout::footprint(&abstractions, comp);
                rect.x += delta[0] as f32;
                rect.y += delta[1] as f32;
                moved_rects.push(rect);
            }
            for other in components.iter().filter(|c| !ids.contains(&c.id)) {
                let other_rect = layout::footprint(&abstractions, other);
                if moved_rects.iter().any(|r| r.intersects(&other_rect)) {
                    return Err(EditError::Overlap);
                }
            }

            for comp in components.iter_mut().filter(|c| ids.contains(&c.id)) {
                comp.pos[0] += delta[0] as f32;
                comp.pos[1] += delta[1] as f32;
            }

            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for id in &note_ids {
                let note = notes
                    .iter_mut()
                    .find(|n| &n.id == id)
                    .ok_or_else(|| EditError::UnknownNote(id.clone()))?;
                note.pos[0] += delta[0] as f32;
                note.pos[1] += delta[1] as f32;
            }
            Ok(())
        })
    }

    /// Delete components and scrub every reference to them from wire nets.
    pub fn delete_components(&mut self, ids: &[String]) -> Result<(), EditError> {
        self.delete_items(ids, &[])
    }

    /// Delete components and notes as one undo step.
    pub fn delete_items(
        &mut self,
        component_ids: &[String],
        note_ids: &[String],
    ) -> Result<(), EditError> {
        self.delete_mixed(component_ids, note_ids, &[], &[])
    }

    /// Delete any mix of selected things — components, notes, wire
    /// segments — and unwire output pins, all in one undo step.
    pub fn delete_mixed(
        &mut self,
        component_ids: &[String],
        note_ids: &[String],
        segments: &[(String, usize)],
        pin_wires: &[String],
    ) -> Result<(), EditError> {
        if component_ids.is_empty()
            && note_ids.is_empty()
            && segments.is_empty()
            && pin_wires.is_empty()
        {
            return Ok(());
        }
        let ids = component_ids.to_vec();
        let note_ids = note_ids.to_vec();
        let segments = segments.to_vec();
        let pin_wires = pin_wires.to_vec();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for id in &ids {
                if !components.iter().any(|c| &c.id == id) {
                    return Err(EditError::UnknownComponent(id.clone()));
                }
            }
            components.retain(|c| !ids.contains(&c.id));

            // Explicitly selected segments go first (indices are valid
            // against the untouched nets), then everything referencing a
            // deleted component.
            for net in wire_nets.iter_mut() {
                let mut indices: Vec<usize> = segments
                    .iter()
                    .filter(|(id, _)| *id == net.id)
                    .map(|(_, i)| *i)
                    .collect();
                indices.sort_unstable();
                indices.dedup();
                for i in indices.into_iter().rev() {
                    if i < net.segments.len() {
                        net.segments.remove(i);
                    }
                }
            }
            for net in wire_nets.iter_mut() {
                net.segments.retain(|seg| {
                    ![&seg.from, &seg.to].iter().any(|e| match e {
                        Endpoint::Port { component_id, .. } => ids.contains(component_id),
                        Endpoint::Junction { .. } => false,
                    })
                });
            }
            repair_nets(wire_nets, components, &ctx)?;

            if !pin_wires.is_empty() {
                let SheetRef::Abstraction(i) = sheet else {
                    return Err(EditError::NotAnAbstractionSheet);
                };
                let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
                for name in &pin_wires {
                    if let Some(target) = def.outputs.get_mut(name) {
                        target.clear();
                    }
                }
            }

            // Output pins that pointed at a deleted component become unwired.
            if let SheetRef::Abstraction(i) = sheet {
                let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
                for target in def.outputs.values_mut() {
                    if !target.is_empty()
                        && ids.contains(&layout::output_target_port(target).component_id)
                    {
                        target.clear();
                    }
                }
            }

            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            notes.retain(|n| !note_ids.contains(&n.id));
            Ok(())
        })
    }

    // ── Notes ────────────────────────────────────────────────────────────

    /// Place a text annotation. Returns its id.
    pub fn place_note(&mut self, pos: [f32; 2], text: &str) -> Result<String, EditError> {
        let text = text.to_string();
        self.mutate(move |project, sheet| {
            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let id = generate_note_id(notes);
            notes.push(crate::project::Note {
                id: id.clone(),
                text: text.clone(),
                pos,
            });
            Ok(id)
        })
    }

    /// Change a note's text. Empty text deletes the note.
    pub fn set_note_text(&mut self, id: &str, text: &str) -> Result<(), EditError> {
        let id = id.to_string();
        let text = text.trim().to_string();
        self.mutate(move |project, sheet| {
            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            if !notes.iter().any(|n| n.id == id) {
                return Err(EditError::UnknownNote(id.clone()));
            }
            if text.is_empty() {
                notes.retain(|n| n.id != id);
            } else {
                notes.iter_mut().find(|n| n.id == id).unwrap().text = text.clone();
            }
            Ok(())
        })
    }

    /// Connect two ports with a wire, optionally bending through `bends`
    /// (grid points, in order from `a` to `b`). One port must be an output
    /// and the other an input, in either argument order. If the input is
    /// already driven, the old connection is replaced.
    ///
    /// This is a convenience over [`Editor::add_wire`]: the whole path is
    /// one undo step. Inside an abstraction sheet, `INPUT:<name>` pins act
    /// as outputs and `OUTPUT:<name>` pins as inputs.
    pub fn connect(
        &mut self,
        a: PortRef,
        b: PortRef,
        bends: &[[f32; 2]],
    ) -> Result<(), EditError> {
        let bends = bends.to_vec();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, _) = sheet_of(project, sheet).ok_or(EditError::BadSheet)?;

            // Draw from the output side so bend order matches wire flow.
            let (from, to, bends) = match (
                ctx.kind_of(components, &a)?,
                ctx.kind_of(components, &b)?,
            ) {
                (PortKind::Output, PortKind::Input) => (a, b, bends),
                (PortKind::Input, PortKind::Output) => {
                    let mut rev = bends.clone();
                    rev.reverse();
                    (b, a, rev)
                }
                _ => return Err(EditError::NotAnInputAndOutput),
            };

            let mut anchor = WirePoint::Port(from);
            for bend in &bends {
                let attach = attach_wire(project, sheet, &anchor, &WirePoint::New(*bend))?;
                anchor = WirePoint::Junction {
                    net_id: attach.net_id,
                    junction_id: attach
                        .new_junction
                        .expect("New points always create a junction"),
                };
            }
            attach_wire(project, sheet, &anchor, &WirePoint::Port(to))?;
            Ok(())
        })
    }

    /// One wire-drawing step: join two points with a straight segment.
    /// Points can be ports, existing wire nodes, or a new node at a grid
    /// position. Nets are created, extended and merged as needed; wiring an
    /// already-connected input replaces its old connection. Each call is one
    /// undo step, so Ctrl+Z peels wires back click by click.
    pub fn add_wire(&mut self, from: WirePoint, to: WirePoint) -> Result<WireAttach, EditError> {
        self.mutate(move |project, sheet| attach_wire(project, sheet, &from, &to))
    }

    /// Move a wire node to a new grid position.
    pub fn move_junction(
        &mut self,
        net_id: &str,
        junction_id: &str,
        pos: [f32; 2],
    ) -> Result<(), EditError> {
        {
            // A no-op move should not spend an undo step.
            let (_, wire_nets) = self.sheet_data();
            let current = wire_nets
                .iter()
                .find(|n| n.id == net_id)
                .and_then(|n| n.junctions.iter().find(|j| j.id == junction_id))
                .map(|j| j.pos);
            if current == Some(pos) {
                return Ok(());
            }
        }
        let net_id = net_id.to_string();
        let junction_id = junction_id.to_string();
        self.mutate(move |project, sheet| {
            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let junction = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .and_then(|n| n.junctions.iter_mut().find(|j| j.id == junction_id))
                .ok_or_else(|| EditError::UnknownJunction(junction_id.clone()))?;
            junction.pos = pos;
            Ok(())
        })
    }

    /// Delete a wire node together with the segments that met there. If that
    /// cuts the net into disconnected pieces, each piece becomes its own net.
    pub fn delete_junction(&mut self, net_id: &str, junction_id: &str) -> Result<(), EditError> {
        let net_id = net_id.to_string();
        let junction_id = junction_id.to_string();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let net = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .ok_or_else(|| EditError::UnknownJunction(junction_id.clone()))?;
            if !net.junctions.iter().any(|j| j.id == junction_id) {
                return Err(EditError::UnknownJunction(junction_id.clone()));
            }
            net.junctions.retain(|j| j.id != junction_id);
            net.segments.retain(|seg| {
                ![&seg.from, &seg.to].iter().any(|e| {
                    matches!(e, Endpoint::Junction { junction_id: id } if *id == junction_id)
                })
            });
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Delete one wire segment. If that cuts the net into disconnected
    /// pieces, each piece becomes its own net.
    pub fn delete_segment(
        &mut self,
        net_id: &str,
        segment_index: usize,
    ) -> Result<(), EditError> {
        let net_id = net_id.to_string();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let net = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .ok_or_else(|| EditError::UnknownNet(net_id.clone()))?;
            if segment_index >= net.segments.len() {
                return Err(EditError::NoopWire);
            }
            net.segments.remove(segment_index);
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Split a segment by inserting a node at `pos` — the wire then runs
    /// through it and new branches can grow from it. Returns the node's id.
    pub fn insert_junction(
        &mut self,
        net_id: &str,
        segment_index: usize,
        pos: [f32; 2],
    ) -> Result<String, EditError> {
        let net_id = net_id.to_string();
        self.mutate(move |project, sheet| {
            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let net = wire_nets
                .iter_mut()
                .find(|n| n.id == net_id)
                .ok_or_else(|| EditError::UnknownNet(net_id.clone()))?;
            if segment_index >= net.segments.len() {
                return Err(EditError::NoopWire);
            }
            let seg = net.segments.remove(segment_index);
            let junction_id = generate_junction_id(net);
            net.junctions.push(Junction {
                id: junction_id.clone(),
                pos,
            });
            let mid = Endpoint::Junction {
                junction_id: junction_id.clone(),
            };
            net.segments.push(crate::project::Segment {
                from: seg.from,
                to: mid.clone(),
            });
            net.segments.push(crate::project::Segment {
                from: mid,
                to: seg.to,
            });
            Ok(junction_id)
        })
    }

    /// Remove one input's connection (the rest of the net stays, including
    /// any nodes that now dangle as reusable stubs).
    pub fn disconnect_sink(&mut self, sink: PortRef) -> Result<(), EditError> {
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            detach_port(wire_nets, &sink);
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Delete several wire segments (one undo step). Nets that fall apart
    /// split into their live pieces.
    pub fn delete_segments(&mut self, items: &[(String, usize)]) -> Result<(), EditError> {
        self.delete_mixed(&[], &[], items, &[])
    }

    /// Unwire output pins (their derived wires disappear; the pins stay).
    pub fn unwire_output_pins(&mut self, names: &[String]) -> Result<(), EditError> {
        self.delete_mixed(&[], &[], &[], names)
    }

    /// Flip a flip-flop's initial value.
    pub fn toggle_dff_init(&mut self, id: &str) -> Result<bool, EditError> {
        let id = id.to_string();
        self.mutate(move |project, sheet| {
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let comp = components
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| EditError::UnknownComponent(id.clone()))?;
            if comp.component_type != "dff" && comp.component_type != "const" {
                return Err(EditError::NotADff(id.clone()));
            }
            let new = !comp.init.unwrap_or(false);
            comp.init = Some(new);
            Ok(new)
        })
    }

    // ── Clipboard fragments ──────────────────────────────────────────────

    /// A self-contained snippet of a sheet: components, notes, and the
    /// wiring that runs among them. What Ctrl+C carries around.
    pub fn fragment(&self, component_ids: &[String], note_ids: &[String]) -> Fragment {
        let (components, wire_nets) = self.sheet_data();
        let components: Vec<ProjectComponent> = components
            .iter()
            .filter(|c| component_ids.contains(&c.id))
            .cloned()
            .collect();
        let notes: Vec<crate::project::Note> = self
            .sheet_notes()
            .iter()
            .filter(|n| note_ids.contains(&n.id))
            .cloned()
            .collect();

        // Keep only wiring fully inside the copied set: segments whose port
        // endpoints all reference copied components. Interface pins are NOT
        // part of the selection, so wires to them are cut like any other
        // outside connection (junction endpoints come along when used).
        let mut nets = Vec::new();
        for net in wire_nets {
            let mut kept = net.clone();
            kept.segments.retain(|seg| {
                [&seg.from, &seg.to].iter().all(|e| match e {
                    Endpoint::Port { component_id, .. } => {
                        component_ids.contains(component_id)
                    }
                    Endpoint::Junction { .. } => true,
                })
            });
            drop_orphan_junctions(&mut kept);
            if !kept.segments.is_empty() {
                nets.push(kept);
            }
        }
        Fragment {
            components,
            notes,
            wire_nets: nets,
        }
    }

    /// Insert a fragment's copies, offset by `delta` grid cells, with fresh
    /// ids everywhere. With `search`, nearby offsets are tried until the
    /// copies fit. Returns the new (component ids, note ids).
    pub fn paste_fragment(
        &mut self,
        fragment: &Fragment,
        delta: [i32; 2],
        search: bool,
    ) -> Result<(Vec<String>, Vec<String>), EditError> {
        let fragment = fragment.clone();
        self.mutate(move |project, sheet| {
            let ctx = KindCtx::new(project, sheet)?;
            let abstractions = project.abstractions.clone();

            // Types must exist here, and pasting an abstraction into its
            // own (or a using) sheet would recurse.
            for comp in &fragment.components {
                if layout::type_ports(&abstractions, &comp.component_type).is_none() {
                    return Err(EditError::UnknownComponentType(
                        comp.component_type.clone(),
                    ));
                }
                if let SheetRef::Abstraction(i) = sheet {
                    let host = abstractions.get(i).ok_or(EditError::BadSheet)?.name.clone();
                    if abstraction_uses(&abstractions, &comp.component_type, &host) {
                        return Err(EditError::RecursiveAbstraction(
                            comp.component_type.clone(),
                        ));
                    }
                }
            }

            // Find an offset where every copy fits.
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let fits = |components: &[ProjectComponent], d: [f32; 2]| -> bool {
                fragment.components.iter().all(|c| {
                    let mut r = layout::footprint(&abstractions, c);
                    r.x += d[0];
                    r.y += d[1];
                    components
                        .iter()
                        .all(|other| !layout::footprint(&abstractions, other).intersects(&r))
                })
            };
            let base = [delta[0] as f32, delta[1] as f32];
            let mut offset = None;
            if fits(components, base) {
                offset = Some(base);
            } else if search {
                'out: for ring in 1..60 {
                    let s = ring as f32;
                    for cand in [
                        [base[0] + s, base[1] + s],
                        [base[0] + s, base[1]],
                        [base[0], base[1] + s],
                        [base[0] - s, base[1]],
                        [base[0], base[1] - s],
                    ] {
                        if fits(components, cand) {
                            offset = Some(cand);
                            break 'out;
                        }
                    }
                }
            }
            let offset = offset.ok_or(EditError::Overlap)?;

            // Fresh component ids, remembering the mapping for the wires.
            let mut id_map: HashMap<String, String> = HashMap::new();
            let mut new_component_ids = Vec::new();
            for original in &fragment.components {
                let id = generate_id(components, &original.component_type);
                id_map.insert(original.id.clone(), id.clone());
                components.push(ProjectComponent {
                    id: id.clone(),
                    component_type: original.component_type.clone(),
                    init: original.init,
                    pos: [original.pos[0] + offset[0], original.pos[1] + offset[1]],
                });
                new_component_ids.push(id);
            }

            // Wires: fresh net + junction ids, ports remapped, everything
            // shifted; logical fields re-derived by the repair pass.
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for net in &fragment.wire_nets {
                let mut copy = net.clone();
                if copy.segments.is_empty() {
                    continue;
                }
                copy.id = generate_net_id(wire_nets);
                copy.source = None;
                copy.sinks.clear();
                for junction in &mut copy.junctions {
                    junction.pos[0] += offset[0];
                    junction.pos[1] += offset[1];
                }
                for seg in &mut copy.segments {
                    for e in [&mut seg.from, &mut seg.to] {
                        if let Endpoint::Port { component_id, .. } = e {
                            if let Some(new_id) = id_map.get(component_id) {
                                *component_id = new_id.clone();
                            }
                        }
                    }
                }
                wire_nets.push(copy);
            }
            repair_nets(wire_nets, components, &ctx)?;

            let notes = sheet_notes_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let mut new_note_ids = Vec::new();
            for original in &fragment.notes {
                let id = generate_note_id(notes);
                notes.push(crate::project::Note {
                    id: id.clone(),
                    text: original.text.clone(),
                    pos: [original.pos[0] + offset[0], original.pos[1] + offset[1]],
                });
                new_note_ids.push(id);
            }

            Ok((new_component_ids, new_note_ids))
        })
    }

    // ── Abstraction interface ────────────────────────────────────────────

    /// Create a new, empty abstraction type. Returns its index.
    pub fn create_abstraction(&mut self, name: &str) -> Result<usize, EditError> {
        let name = name.to_string();
        self.mutate(move |project, _| {
            if !valid_name(&name) {
                return Err(EditError::InvalidName(name.clone()));
            }
            if crate::types::is_primitive(&name)
                || name == "note"
                || project.abstractions.iter().any(|a| a.name == name)
            {
                return Err(EditError::NameTaken(name.clone()));
            }
            let [width, height] = layout::default_abstraction_size(&name, 0, 0);
            project.abstractions.push(crate::project::ProjectAbstraction {
                pin_positions: HashMap::new(),
                port_layout: HashMap::new(),
                name: name.clone(),
                inputs: vec![],
                outputs: HashMap::new(),
                width,
                height,
                components: vec![],
                wire_nets: vec![],
                notes: vec![],
            });
            Ok(project.abstractions.len() - 1)
        })
    }

    /// Add an input or output pin to the abstraction being edited.
    /// An output pin starts unwired (empty target) until connected.
    pub fn add_interface_pin(&mut self, kind: PortKind, name: &str) -> Result<(), EditError> {
        let name = name.to_string();
        self.mutate(move |project, sheet| {
            let SheetRef::Abstraction(i) = sheet else {
                return Err(EditError::NotAnAbstractionSheet);
            };
            let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
            if !valid_name(&name) {
                return Err(EditError::InvalidName(name.clone()));
            }
            if def.inputs.iter().any(|x| x == &name) || def.outputs.contains_key(&name) {
                return Err(EditError::NameTaken(name.clone()));
            }
            let index = match kind {
                PortKind::Input => {
                    def.inputs.push(name.clone());
                    def.inputs.len() - 1
                }
                PortKind::Output => {
                    def.outputs.insert(name.clone(), String::new());
                    def.outputs.len() - 1
                }
            };
            ensure_interface_fits(def);

            // Hand-layouted components stay hand-layouted: park the new
            // port in a free half-grid slot on its default side.
            if !def.port_layout.is_empty() {
                use crate::project::{PortSide, PortSpot};
                let side = match kind {
                    PortKind::Input => PortSide::Left,
                    PortKind::Output => PortSide::Right,
                };
                let len = def.height;
                let occupied: Vec<i64> = def
                    .port_layout
                    .values()
                    .filter(|s| s.side == side)
                    .map(|s| (s.t * len - 0.5).round() as i64)
                    .collect();
                let mut slot = (0..len as i64).find(|k| !occupied.contains(k));
                if slot.is_none() {
                    def.height += 1.0;
                    slot = Some(def.height as i64 - 1);
                }
                let len = def.height;
                def.port_layout.insert(
                    name.clone(),
                    PortSpot {
                        side,
                        t: (slot.unwrap() as f32 + 0.5) / len,
                    },
                );
            }

            // Pins are free-floating objects: give the new one a concrete
            // home on the rail so it never drifts as content changes.
            let abstractions = project.abstractions.clone();
            let pos =
                layout::default_pin_position(&abstractions, &abstractions[i], kind, index);
            project.abstractions[i].pin_positions.insert(name.clone(), pos);
            Ok(())
        })
    }

    /// Move an interface pin to a new spot inside its sheet.
    pub fn set_pin_position(&mut self, name: &str, pos: [f32; 2]) -> Result<(), EditError> {
        let name = name.to_string();
        self.mutate(move |project, sheet| {
            let SheetRef::Abstraction(i) = sheet else {
                return Err(EditError::NotAnAbstractionSheet);
            };
            let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
            if !def.inputs.iter().any(|x| x == &name) && !def.outputs.contains_key(&name) {
                return Err(EditError::UnknownComponent(name.clone()));
            }
            def.pin_positions.insert(name.clone(), pos);
            Ok(())
        })
    }

    /// Remove a pin, scrubbing every wire that used it.
    pub fn remove_interface_pin(&mut self, kind: PortKind, name: &str) -> Result<(), EditError> {
        let name = name.to_string();
        self.mutate(move |project, sheet| {
            let SheetRef::Abstraction(i) = sheet else {
                return Err(EditError::NotAnAbstractionSheet);
            };
            let abstractions_snapshot = project.abstractions.clone();
            let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
            match kind {
                PortKind::Input => {
                    if !def.inputs.iter().any(|x| x == &name) {
                        return Err(EditError::UnknownComponent(format!(
                            "{}{name}",
                            crate::convert::INPUT_PREFIX
                        )));
                    }
                    def.inputs.retain(|x| x != &name);
                    let pseudo_id = format!("{}{name}", crate::convert::INPUT_PREFIX);
                    for net in def.wire_nets.iter_mut() {
                        net.segments.retain(|seg| {
                            ![&seg.from, &seg.to].iter().any(|e| {
                                matches!(e, Endpoint::Port { component_id, .. }
                                    if *component_id == pseudo_id)
                            })
                        });
                    }
                    let ctx = KindCtx {
                        abstractions: abstractions_snapshot,
                        interface_inputs: Some(def.inputs.clone()),
                        interface_outputs: Some(def.outputs.keys().cloned().collect()),
                    };
                    let components = def.components.clone();
                    repair_nets(&mut def.wire_nets, &components, &ctx)?;
                }
                PortKind::Output => {
                    if def.outputs.remove(&name).is_none() {
                        return Err(EditError::UnknownComponent(format!(
                            "{}{name}",
                            layout::OUTPUT_PREFIX
                        )));
                    }
                }
            }
            let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
            def.pin_positions.remove(&name);
            def.port_layout.remove(&name);
            ensure_interface_fits(def);
            let abs_name = def.name.clone();

            // The pin's port existed on every instance: scrub the wires
            // attached to it on every sheet, or those sheets would be left
            // with dangling references that poison later operations.
            let abstractions_snapshot = project.abstractions.clone();
            let sheet_count = project.abstractions.len() + 1;
            for sheet_index in 0..sheet_count {
                let (sheet_components, sheet_nets) = if sheet_index == 0 {
                    (&project.components, &mut project.wire_nets)
                } else {
                    let a = &mut project.abstractions[sheet_index - 1];
                    (&a.components, &mut a.wire_nets)
                };
                let instances: Vec<String> = sheet_components
                    .iter()
                    .filter(|c| c.component_type == abs_name)
                    .map(|c| c.id.clone())
                    .collect();
                if instances.is_empty() {
                    continue;
                }
                for net in sheet_nets.iter_mut() {
                    net.segments.retain(|seg| {
                        ![&seg.from, &seg.to].iter().any(|e| {
                            matches!(e, Endpoint::Port { component_id, port_name }
                                if instances.contains(component_id) && *port_name == name)
                        })
                    });
                }
                let components_snapshot = sheet_components.clone();
                let interface = if sheet_index == 0 {
                    None
                } else {
                    abstractions_snapshot.get(sheet_index - 1)
                };
                let ctx = KindCtx {
                    abstractions: abstractions_snapshot.clone(),
                    interface_inputs: interface.map(|a| a.inputs.clone()),
                    interface_outputs: interface
                        .map(|a| a.outputs.keys().cloned().collect()),
                };
                let sheet_nets = if sheet_index == 0 {
                    &mut project.wire_nets
                } else {
                    &mut project.abstractions[sheet_index - 1].wire_nets
                };
                repair_nets(sheet_nets, &components_snapshot, &ctx)?;
            }

            // Other abstractions' output pins may have dotted into this
            // pin on an instance ("inst.pin"): unwire them.
            if kind == PortKind::Output {
                for a in project.abstractions.iter_mut() {
                    let instances: Vec<String> = a
                        .components
                        .iter()
                        .filter(|c| c.component_type == abs_name)
                        .map(|c| c.id.clone())
                        .collect();
                    if instances.is_empty() {
                        continue;
                    }
                    for target in a.outputs.values_mut() {
                        if target.is_empty() {
                            continue;
                        }
                        let port = layout::output_target_port(target);
                        if instances.contains(&port.component_id) && port.port_name == name {
                            target.clear();
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Place a port on a chosen edge of the closed rectangle.
    pub fn set_port_layout(
        &mut self,
        index: usize,
        name: &str,
        side: crate::project::PortSide,
        t: f32,
    ) -> Result<(), EditError> {
        let name = name.to_string();
        self.mutate(move |project, _| {
            let abstractions = project.abstractions.clone();
            let def = project
                .abstractions
                .get_mut(index)
                .ok_or(EditError::BadSheet)?;
            if !def.inputs.iter().any(|x| x == &name) && !def.outputs.contains_key(&name) {
                return Err(EditError::UnknownComponent(name.clone()));
            }
            // The first hand-placed port freezes every other port exactly
            // where the auto layout has it — nothing jumps.
            if def.port_layout.is_empty() {
                for (port, spot_side, spot_t) in
                    layout::current_port_spots(&abstractions, &abstractions[index])
                {
                    def.port_layout.insert(
                        port,
                        crate::project::PortSpot {
                            side: spot_side,
                            t: spot_t,
                        },
                    );
                }
            }
            def.port_layout.insert(
                name.clone(),
                crate::project::PortSpot {
                    side,
                    t: t.clamp(0.0, 1.0),
                },
            );
            ensure_interface_fits(def);
            Ok(())
        })
    }

    /// Resize an abstraction's closed rectangle.
    pub fn set_abstraction_size(&mut self, index: usize, w: f32, h: f32) -> Result<(), EditError> {
        self.mutate(move |project, _| {
            let def = project
                .abstractions
                .get_mut(index)
                .ok_or(EditError::BadSheet)?;
            def.width = w.clamp(2.0, 40.0);
            def.height = h.clamp(1.5, 40.0);
            Ok(())
        })
    }

    /// Rename a pin of the abstraction being edited, rewriting every
    /// reference: its own sheet's wires, the wires attached to instances on
    /// every other sheet, and outputs-map targets that dot into instances.
    pub fn rename_interface_pin(
        &mut self,
        kind: PortKind,
        old: &str,
        new: &str,
    ) -> Result<(), EditError> {
        if old == new {
            return Ok(());
        }
        let old = old.to_string();
        let new = new.to_string();
        self.mutate(move |project, sheet| {
            let SheetRef::Abstraction(index) = sheet else {
                return Err(EditError::NotAnAbstractionSheet);
            };
            if !valid_name(&new) {
                return Err(EditError::InvalidName(new.clone()));
            }
            let def = project.abstractions.get_mut(index).ok_or(EditError::BadSheet)?;
            if def.inputs.iter().any(|x| x == &new) || def.outputs.contains_key(&new) {
                return Err(EditError::NameTaken(new.clone()));
            }
            let abs_name = def.name.clone();

            if let Some(pos) = def.pin_positions.remove(&old) {
                def.pin_positions.insert(new.clone(), pos);
            }
            if let Some(spot) = def.port_layout.remove(&old) {
                def.port_layout.insert(new.clone(), spot);
            }
            match kind {
                PortKind::Input => {
                    let slot = def
                        .inputs
                        .iter_mut()
                        .find(|x| *x == &old)
                        .ok_or_else(|| EditError::UnknownComponent(old.clone()))?;
                    *slot = new.clone();
                    // The pin's pseudo-port inside its own sheet.
                    let old_id = format!("{}{old}", crate::convert::INPUT_PREFIX);
                    let new_id = format!("{}{new}", crate::convert::INPUT_PREFIX);
                    let rewrite = |p: &mut PortRef| {
                        if p.component_id == old_id {
                            p.component_id = new_id.clone();
                            p.port_name = new.clone();
                        }
                    };
                    for net in def.wire_nets.iter_mut() {
                        if let Some(s) = &mut net.source {
                            rewrite(s);
                        }
                        net.sinks.iter_mut().for_each(&rewrite);
                        for seg in &mut net.segments {
                            for e in [&mut seg.from, &mut seg.to] {
                                if let Endpoint::Port {
                                    component_id,
                                    port_name,
                                } = e
                                {
                                    if *component_id == old_id {
                                        *component_id = new_id.clone();
                                        *port_name = new.clone();
                                    }
                                }
                            }
                        }
                    }
                }
                PortKind::Output => {
                    let target = def
                        .outputs
                        .remove(&old)
                        .ok_or_else(|| EditError::UnknownComponent(old.clone()))?;
                    def.outputs.insert(new.clone(), target);
                }
            }

            // The pin's port on every instance, on every sheet.
            let sheet_count = project.abstractions.len() + 1;
            for sheet_index in 0..sheet_count {
                let (components, wire_nets) = if sheet_index == 0 {
                    (&project.components, &mut project.wire_nets)
                } else {
                    let a = &mut project.abstractions[sheet_index - 1];
                    (&a.components, &mut a.wire_nets)
                };
                let instances: Vec<String> = components
                    .iter()
                    .filter(|c| c.component_type == abs_name)
                    .map(|c| c.id.clone())
                    .collect();
                if instances.is_empty() {
                    continue;
                }
                for net in wire_nets.iter_mut() {
                    let rewrite = |p: &mut PortRef| {
                        if instances.contains(&p.component_id) && p.port_name == old {
                            p.port_name = new.clone();
                        }
                    };
                    if let Some(s) = &mut net.source {
                        rewrite(s);
                    }
                    net.sinks.iter_mut().for_each(rewrite);
                    for seg in &mut net.segments {
                        for e in [&mut seg.from, &mut seg.to] {
                            if let Endpoint::Port {
                                component_id,
                                port_name,
                            } = e
                            {
                                if instances.contains(component_id) && *port_name == old {
                                    *port_name = new.clone();
                                }
                            }
                        }
                    }
                }
            }

            // Renamed OUTPUT pins can appear in other abstractions' outputs
            // maps as "instance.old" dot targets.
            if kind == PortKind::Output {
                for a in project.abstractions.iter_mut() {
                    let instances: Vec<String> = a
                        .components
                        .iter()
                        .filter(|c| c.component_type == abs_name)
                        .map(|c| c.id.clone())
                        .collect();
                    if instances.is_empty() {
                        continue;
                    }
                    for target in a.outputs.values_mut() {
                        let port = layout::output_target_port(target);
                        if instances.contains(&port.component_id) && port.port_name == old {
                            *target = format!("{}.{new}", port.component_id);
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Rename the abstraction being edited, retyping every instance.
    pub fn rename_abstraction(&mut self, index: usize, new_name: &str) -> Result<(), EditError> {
        let new_name = new_name.to_string();
        self.mutate(move |project, _| {
            let old_name = project
                .abstractions
                .get(index)
                .ok_or(EditError::BadSheet)?
                .name
                .clone();
            if new_name == old_name {
                return Ok(());
            }
            if !valid_name(&new_name) {
                return Err(EditError::InvalidName(new_name.clone()));
            }
            if crate::types::is_primitive(&new_name)
                || new_name == "note"
                || project.abstractions.iter().any(|a| a.name == new_name)
            {
                return Err(EditError::NameTaken(new_name.clone()));
            }
            project.abstractions[index].name = new_name.clone();
            for comp in project.components.iter_mut() {
                if comp.component_type == old_name {
                    comp.component_type = new_name.clone();
                }
            }
            for a in project.abstractions.iter_mut() {
                for comp in a.components.iter_mut() {
                    if comp.component_type == old_name {
                        comp.component_type = new_name.clone();
                    }
                }
            }
            Ok(())
        })
    }

    /// Rename the project (its display title, also the default filename).
    pub fn rename_project(&mut self, name: &str) -> Result<(), EditError> {
        let name = name.trim().to_string();
        self.mutate(move |project, _| {
            if name.is_empty() {
                return Err(EditError::InvalidName(name.clone()));
            }
            project.name = name.clone();
            Ok(())
        })
    }

    /// Delete an abstraction type. Refused while instances of it exist.
    pub fn delete_abstraction(&mut self, index: usize) -> Result<(), EditError> {
        self.mutate(move |project, _| {
            let name = project
                .abstractions
                .get(index)
                .ok_or(EditError::BadSheet)?
                .name
                .clone();
            let uses = project
                .components
                .iter()
                .chain(project.abstractions.iter().flat_map(|a| a.components.iter()))
                .filter(|c| c.component_type == name)
                .count();
            if uses > 0 {
                return Err(EditError::AbstractionInUse(uses));
            }
            project.abstractions.remove(index);
            Ok(())
        })?;
        // The sheet indices above `index` all shifted down by one.
        self.sheet = match self.sheet {
            SheetRef::Abstraction(j) if j == index => SheetRef::Top,
            SheetRef::Abstraction(j) if j > index => SheetRef::Abstraction(j - 1),
            other => other,
        };
        Ok(())
    }
}

/// Grow an abstraction's closed rectangle so every side has a half-grid
/// slot for each of its ports (never shrinks).
fn ensure_interface_fits(def: &mut crate::project::ProjectAbstraction) {
    use crate::project::PortSide;
    let side_of = |name: &str, default: PortSide| -> PortSide {
        def.port_layout.get(name).map(|s| s.side).unwrap_or(default)
    };
    let mut counts = [0usize; 4]; // L R T B
    let slot = |side: PortSide| match side {
        PortSide::Left => 0,
        PortSide::Right => 1,
        PortSide::Top => 2,
        PortSide::Bottom => 3,
    };
    for name in &def.inputs {
        counts[slot(side_of(name, PortSide::Left))] += 1;
    }
    for name in def.outputs.keys() {
        counts[slot(side_of(name, PortSide::Right))] += 1;
    }
    def.height = def.height.max(counts[0].max(counts[1]).max(2) as f32);
    def.width = def.width.max(counts[2].max(counts[3]).max(2) as f32);
}

// ── Sheet plumbing ───────────────────────────────────────────────────────────

pub fn sheet_of(
    project: &Project,
    sheet: SheetRef,
) -> Option<(&[ProjectComponent], &[WireNet])> {
    match sheet {
        SheetRef::Top => Some((&project.components, &project.wire_nets)),
        SheetRef::Abstraction(i) => project
            .abstractions
            .get(i)
            .map(|a| (a.components.as_slice(), a.wire_nets.as_slice())),
    }
}

fn sheet_mut(
    project: &mut Project,
    sheet: SheetRef,
) -> Option<(&mut Vec<ProjectComponent>, &mut Vec<WireNet>)> {
    match sheet {
        SheetRef::Top => Some((&mut project.components, &mut project.wire_nets)),
        SheetRef::Abstraction(i) => project
            .abstractions
            .get_mut(i)
            .map(|a| (&mut a.components, &mut a.wire_nets)),
    }
}

pub fn sheet_notes(project: &Project, sheet: SheetRef) -> Option<&[crate::project::Note]> {
    match sheet {
        SheetRef::Top => Some(project.notes.as_slice()),
        SheetRef::Abstraction(i) => project.abstractions.get(i).map(|a| a.notes.as_slice()),
    }
}

fn sheet_notes_mut(
    project: &mut Project,
    sheet: SheetRef,
) -> Option<&mut Vec<crate::project::Note>> {
    match sheet {
        SheetRef::Top => Some(&mut project.notes),
        SheetRef::Abstraction(i) => project.abstractions.get_mut(i).map(|a| &mut a.notes),
    }
}

/// Does abstraction `name` (transitively) instantiate `target`?
/// Also true when `name == target`. Robust against cyclic definitions in
/// hand-edited files (a visited set bounds the walk).
pub fn abstraction_uses(
    abstractions: &[crate::project::ProjectAbstraction],
    name: &str,
    target: &str,
) -> bool {
    let mut visited: Vec<&str> = Vec::new();
    let mut stack: Vec<&str> = vec![name];
    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if visited.contains(&current) {
            continue;
        }
        visited.push(current);
        if let Some(def) = abstractions.iter().find(|a| a.name == current) {
            for c in &def.components {
                stack.push(&c.component_type);
            }
        }
    }
    false
}

// ── ID generation (always derived from the document, never a counter) ───────

fn generate_id(components: &[ProjectComponent], type_name: &str) -> String {
    let prefix: String = type_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    for n in 1.. {
        let candidate = format!("{prefix}_{n}");
        if !components.iter().any(|c| c.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn generate_net_id(wire_nets: &[WireNet]) -> String {
    for n in 1.. {
        let candidate = format!("net_{n}");
        if !wire_nets.iter().any(|w| w.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn generate_note_id(notes: &[crate::project::Note]) -> String {
    for n in 1.. {
        let candidate = format!("note_{n}");
        if !notes.iter().any(|note| note.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn generate_junction_id(net: &WireNet) -> String {
    for n in 1.. {
        let candidate = format!("j_{n}");
        if !net.junctions.iter().any(|j| j.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

// ── The wire graph model ─────────────────────────────────────────────────────
//
// A net's SEGMENTS are the truth: a net is one connected graph of port
// endpoints and junctions ("nodes"). The logical `source`/`sinks` fields the
// backend reads are derived from the segment endpoints by `sync_net_logic`
// after every mutation. Wires behave like placed objects — stubs and
// unfinished branches are legitimate document state, not junk.

/// A point a wire-drawing step can start or end on.
#[derive(Debug, Clone, PartialEq)]
pub enum WirePoint {
    Port(PortRef),
    Junction {
        net_id: String,
        junction_id: String,
    },
    /// Create a new node at this grid position.
    New([f32; 2]),
}

/// What an [`Editor::add_wire`] step produced.
#[derive(Debug, Clone)]
pub struct WireAttach {
    /// The net the segment landed in. Empty for output-pin connections
    /// (those live in the abstraction's outputs map, not in a net).
    pub net_id: String,
    /// The node created for a `WirePoint::New` target.
    pub new_junction: Option<String>,
}

/// Port-kind resolution context, captured up front so operations can hold
/// mutable borrows of the sheet while classifying ports.
struct KindCtx {
    abstractions: Vec<crate::project::ProjectAbstraction>,
    interface_inputs: Option<Vec<String>>,
    interface_outputs: Option<Vec<String>>,
}

impl KindCtx {
    fn new(project: &Project, sheet: SheetRef) -> Result<Self, EditError> {
        let (interface_inputs, interface_outputs) = match sheet {
            SheetRef::Top => (None, None),
            SheetRef::Abstraction(i) => {
                let def = project.abstractions.get(i).ok_or(EditError::BadSheet)?;
                (
                    Some(def.inputs.clone()),
                    Some(def.outputs.keys().cloned().collect()),
                )
            }
        };
        Ok(KindCtx {
            abstractions: project.abstractions.clone(),
            interface_inputs,
            interface_outputs,
        })
    }

    /// Wire-direction kind of a port, including interface pseudo-ports.
    fn kind_of(
        &self,
        components: &[ProjectComponent],
        port: &PortRef,
    ) -> Result<PortKind, EditError> {
        if let Some(name) = port.component_id.strip_prefix(crate::convert::INPUT_PREFIX) {
            let inputs = self
                .interface_inputs
                .as_ref()
                .ok_or(EditError::NotAnAbstractionSheet)?;
            return if inputs.iter().any(|i| i == name) {
                Ok(PortKind::Output) // input pins drive wires
            } else {
                Err(EditError::UnknownComponent(port.component_id.clone()))
            };
        }
        if let Some(name) = port.component_id.strip_prefix(layout::OUTPUT_PREFIX) {
            let outputs = self
                .interface_outputs
                .as_ref()
                .ok_or(EditError::NotAnAbstractionSheet)?;
            return if outputs.iter().any(|o| o == name) {
                Ok(PortKind::Input) // output pins are fed by wires
            } else {
                Err(EditError::UnknownComponent(port.component_id.clone()))
            };
        }
        let comp = components
            .iter()
            .find(|c| c.id == port.component_id)
            .ok_or_else(|| EditError::UnknownComponent(port.component_id.clone()))?;
        layout::type_ports(&self.abstractions, &comp.component_type)
            .and_then(|ports| ports.into_iter().find(|p| p.name == port.port_name))
            .map(|p| p.kind)
            .ok_or_else(|| {
                EditError::UnknownPort(port.component_id.clone(), port.port_name.clone())
            })
    }
}

fn port_endpoint(port: &PortRef) -> Endpoint {
    Endpoint::Port {
        component_id: port.component_id.clone(),
        port_name: port.port_name.clone(),
    }
}

fn endpoint_is_port(e: &Endpoint, port: &PortRef) -> bool {
    matches!(e, Endpoint::Port { component_id, port_name }
        if *component_id == port.component_id && *port_name == port.port_name)
}

fn net_touches_port(net: &WireNet, port: &PortRef) -> bool {
    net.segments
        .iter()
        .any(|seg| endpoint_is_port(&seg.from, port) || endpoint_is_port(&seg.to, port))
}

/// The single wire-drawing step both `add_wire` and `connect` build on.
fn attach_wire(
    project: &mut Project,
    sheet: SheetRef,
    from: &WirePoint,
    to: &WirePoint,
) -> Result<WireAttach, EditError> {
    let ctx = KindCtx::new(project, sheet)?;

    // Output-pin connections are outputs-map writes, not net edits.
    let is_output_pin = |wp: &WirePoint| {
        matches!(wp, WirePoint::Port(p) if p.component_id.starts_with(layout::OUTPUT_PREFIX))
    };
    if is_output_pin(from) || is_output_pin(to) {
        return attach_to_output_pin(project, sheet, &ctx, from, to);
    }

    if from == to {
        return Err(EditError::NoopWire);
    }

    let (components, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;

    // If this exact wire already exists, it's a no-op — checked before the
    // replace rule below can detach and re-add it.
    let existing_endpoint = |wp: &WirePoint| -> Option<Endpoint> {
        match wp {
            WirePoint::Port(p) => Some(port_endpoint(p)),
            WirePoint::Junction { junction_id, .. } => Some(Endpoint::Junction {
                junction_id: junction_id.clone(),
            }),
            WirePoint::New(_) => None,
        }
    };
    if let (Some(a), Some(b)) = (existing_endpoint(from), existing_endpoint(to)) {
        let already_there = wire_nets.iter().any(|n| {
            n.segments
                .iter()
                .any(|seg| (seg.from == a && seg.to == b) || (seg.from == b && seg.to == a))
        });
        if already_there {
            return Err(EditError::NoopWire);
        }
    }

    // Validate port points and apply the replace rule: wiring an input that
    // is already connected replaces its old connection.
    for wp in [from, to] {
        if let WirePoint::Port(p) = wp {
            if ctx.kind_of(components, p)? == PortKind::Input
                && wire_nets.iter().any(|n| net_touches_port(n, p))
            {
                detach_port(wire_nets, p);
            }
        }
    }

    // Resolve which existing net each point belongs to.
    let resolve_net = |wire_nets: &Vec<WireNet>, wp: &WirePoint| -> Result<Option<usize>, EditError> {
        match wp {
            WirePoint::Port(p) => Ok(wire_nets.iter().position(|n| net_touches_port(n, p))),
            WirePoint::Junction {
                net_id,
                junction_id,
            } => {
                let index = wire_nets
                    .iter()
                    .position(|n| {
                        n.id == *net_id && n.junctions.iter().any(|j| j.id == *junction_id)
                    })
                    .ok_or_else(|| EditError::UnknownJunction(junction_id.clone()))?;
                Ok(Some(index))
            }
            WirePoint::New(_) => Ok(None),
        }
    };

    let net_from = resolve_net(wire_nets, from)?;
    let net_to = resolve_net(wire_nets, to)?;

    // Junction ids can be renamed if a merge collides; track that.
    let mut renames: Vec<(String, String)> = Vec::new();
    let net_index = match (net_from, net_to) {
        (Some(a), Some(b)) if a != b => {
            // Merging two live nets: at most one may be driven.
            if let (Some(sa), Some(sb)) = (&wire_nets[a].source, &wire_nets[b].source) {
                if sa != sb {
                    return Err(EditError::TwoSources(format!(
                        "{}.{}",
                        sa.component_id, sa.port_name
                    )));
                }
            }
            let absorbed = wire_nets.remove(b);
            let a = if b < a { a - 1 } else { a };
            renames = merge_net(&mut wire_nets[a], absorbed);
            a
        }
        (Some(a), _) => a,
        (_, Some(b)) => b,
        (None, None) => {
            let id = generate_net_id(wire_nets);
            wire_nets.push(WireNet {
                id,
                source: None,
                sinks: vec![],
                junctions: vec![],
                segments: vec![],
            });
            wire_nets.len() - 1
        }
    };

    let mut new_junction = None;
    let mut endpoint_of = |wp: &WirePoint, net: &mut WireNet| -> Result<Endpoint, EditError> {
        match wp {
            WirePoint::Port(p) => Ok(port_endpoint(p)),
            WirePoint::Junction { junction_id, .. } => {
                let id = renames
                    .iter()
                    .find(|(old, _)| old == junction_id)
                    .map(|(_, new)| new.clone())
                    .unwrap_or_else(|| junction_id.clone());
                if !net.junctions.iter().any(|j| j.id == id) {
                    return Err(EditError::UnknownJunction(id));
                }
                Ok(Endpoint::Junction { junction_id: id })
            }
            WirePoint::New(pos) => {
                let id = generate_junction_id(net);
                net.junctions.push(Junction {
                    id: id.clone(),
                    pos: *pos,
                });
                new_junction = Some(id.clone());
                Ok(Endpoint::Junction { junction_id: id })
            }
        }
    };

    let net = &mut wire_nets[net_index];
    let from_ep = endpoint_of(from, net)?;
    let to_ep = endpoint_of(to, net)?;

    let duplicate = net.segments.iter().any(|seg| {
        (seg.from == from_ep && seg.to == to_ep) || (seg.from == to_ep && seg.to == from_ep)
    });
    if duplicate {
        return Err(EditError::NoopWire);
    }
    net.segments.push(crate::project::Segment {
        from: from_ep,
        to: to_ep,
    });

    let net_id = net.id.clone();
    // Re-derive logic; this is also where two-sources and doubly-wired
    // inputs are rejected (the whole step rolls back on error).
    for net in wire_nets.iter_mut() {
        sync_net_logic(net, components, &ctx)?;
        drop_orphan_junctions(net);
    }
    gc_nets(wire_nets);

    Ok(WireAttach {
        net_id,
        new_junction,
    })
}

/// Wiring into an `OUTPUT:` pin records the driving port in the
/// abstraction's outputs map (the wire itself is drawn derived from it).
fn attach_to_output_pin(
    project: &mut Project,
    sheet: SheetRef,
    ctx: &KindCtx,
    from: &WirePoint,
    to: &WirePoint,
) -> Result<WireAttach, EditError> {
    let SheetRef::Abstraction(index) = sheet else {
        return Err(EditError::NotAnAbstractionSheet);
    };
    let is_pin = |wp: &WirePoint| {
        matches!(wp, WirePoint::Port(p) if p.component_id.starts_with(layout::OUTPUT_PREFIX))
    };
    let (pin, other) = if is_pin(from) { (from, to) } else { (to, from) };
    let WirePoint::Port(pin_port) = pin else {
        unreachable!("is_pin checked")
    };
    let pin_name = pin_port
        .component_id
        .strip_prefix(layout::OUTPUT_PREFIX)
        .expect("checked prefix")
        .to_string();

    let (components, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
    if !ctx
        .interface_outputs
        .as_ref()
        .is_some_and(|outs| outs.contains(&pin_name))
    {
        return Err(EditError::UnknownComponent(pin_port.component_id.clone()));
    }

    // Find the port that will drive this pin.
    let source_port: PortRef = match other {
        WirePoint::Port(p) => {
            if p.component_id.starts_with(crate::convert::INPUT_PREFIX) {
                return Err(EditError::PassthroughNotAllowed);
            }
            if ctx.kind_of(components, p)? != PortKind::Output {
                return Err(EditError::NotAnInputAndOutput);
            }
            p.clone()
        }
        WirePoint::Junction { net_id, .. } => {
            let net = wire_nets
                .iter()
                .find(|n| &n.id == net_id)
                .ok_or_else(|| EditError::UnknownJunction(net_id.clone()))?;
            let source = net
                .source
                .clone()
                .ok_or(EditError::OutputPinNeedsSource)?;
            if source.component_id.starts_with(crate::convert::INPUT_PREFIX) {
                return Err(EditError::PassthroughNotAllowed);
            }
            source
        }
        WirePoint::New(_) => return Err(EditError::OutputPinNeedsSource),
    };

    let source_comp = components
        .iter()
        .find(|c| c.id == source_port.component_id)
        .ok_or_else(|| EditError::UnknownComponent(source_port.component_id.clone()))?;
    let target = if crate::types::is_primitive(&source_comp.component_type) {
        source_comp.id.clone()
    } else {
        format!("{}.{}", source_port.component_id, source_port.port_name)
    };
    project.abstractions[index].outputs.insert(pin_name, target);
    Ok(WireAttach {
        net_id: String::new(),
        new_junction: None,
    })
}

/// Remove every segment touching `port`, across all nets. Orphaned nodes and
/// empty nets are left for `repair_nets`/callers to tidy.
fn detach_port(wire_nets: &mut [WireNet], port: &PortRef) {
    for net in wire_nets.iter_mut() {
        net.segments
            .retain(|seg| !endpoint_is_port(&seg.from, port) && !endpoint_is_port(&seg.to, port));
    }
}

/// Derive a net's logical `source`/`sinks` from its segment endpoints.
/// Rejects nets with two different driving outputs or an input wired twice.
fn sync_net_logic(
    net: &mut WireNet,
    components: &[ProjectComponent],
    ctx: &KindCtx,
) -> Result<(), EditError> {
    let mut source: Option<PortRef> = None;
    let mut sinks: Vec<PortRef> = Vec::new();
    for seg in &net.segments {
        for e in [&seg.from, &seg.to] {
            let Endpoint::Port {
                component_id,
                port_name,
            } = e
            else {
                continue;
            };
            let port = PortRef {
                component_id: component_id.clone(),
                port_name: port_name.clone(),
            };
            match ctx.kind_of(components, &port)? {
                PortKind::Output => match &source {
                    None => source = Some(port),
                    Some(s) if *s == port => {}
                    Some(s) => {
                        return Err(EditError::TwoSources(format!(
                            "{}.{}",
                            s.component_id, s.port_name
                        )))
                    }
                },
                PortKind::Input => {
                    if sinks.contains(&port) {
                        return Err(EditError::InputDoublyWired(format!(
                            "{}.{}",
                            port.component_id, port.port_name
                        )));
                    }
                    sinks.push(port);
                }
            }
        }
    }
    net.source = source;
    net.sinks = sinks;
    Ok(())
}

/// Drop nodes no segment touches (invisible floating dots).
fn drop_orphan_junctions(net: &mut WireNet) {
    net.junctions.retain(|j| {
        net.segments.iter().any(|seg| {
            [&seg.from, &seg.to].iter().any(
                |e| matches!(e, Endpoint::Junction { junction_id } if *junction_id == j.id),
            )
        })
    });
}

fn endpoint_key(e: &Endpoint) -> String {
    match e {
        Endpoint::Port {
            component_id,
            port_name,
        } => format!("P:{component_id}\u{0}{port_name}"),
        Endpoint::Junction { junction_id } => format!("J:{junction_id}"),
    }
}

/// A net must be one connected graph: if its segments form several islands,
/// split the extras into their own nets.
fn split_all_nets(wire_nets: &mut Vec<WireNet>) {
    let mut i = 0;
    while i < wire_nets.len() {
        for (junctions, segments) in split_off_islands(&mut wire_nets[i]) {
            let id = generate_net_id(wire_nets);
            wire_nets.push(WireNet {
                id,
                source: None, // synced by the caller
                sinks: vec![],
                junctions,
                segments,
            });
        }
        i += 1;
    }
}

/// Remove all but the first connected component from `net`, returning the
/// (junctions, segments) of each removed island.
fn split_off_islands(
    net: &mut WireNet,
) -> Vec<(Vec<Junction>, Vec<crate::project::Segment>)> {
    if net.segments.len() <= 1 {
        return vec![];
    }

    // Group segments into connected components over endpoint keys.
    let mut groups: Vec<(Vec<String>, Vec<crate::project::Segment>)> = Vec::new();
    for seg in net.segments.drain(..) {
        let ka = endpoint_key(&seg.from);
        let kb = endpoint_key(&seg.to);
        let touching: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, (keys, _))| keys.contains(&ka) || keys.contains(&kb))
            .map(|(g, _)| g)
            .collect();
        match touching.as_slice() {
            [] => groups.push((vec![ka, kb], vec![seg])),
            [g] => {
                let (keys, segs) = &mut groups[*g];
                keys.push(ka);
                keys.push(kb);
                segs.push(seg);
            }
            [first, rest @ ..] => {
                // This segment bridges several groups: merge them.
                for &g in rest.iter().rev() {
                    let (keys, segs) = groups.remove(g);
                    let target = &mut groups[*first];
                    target.0.extend(keys);
                    target.1.extend(segs);
                }
                let target = &mut groups[*first];
                target.0.push(ka);
                target.0.push(kb);
                target.1.push(seg);
            }
        }
    }

    // First group stays in this net; junctions follow their group.
    let all_junctions = std::mem::take(&mut net.junctions);
    let mut islands = Vec::new();
    for (index, (keys, segments)) in groups.into_iter().enumerate() {
        let junctions: Vec<Junction> = all_junctions
            .iter()
            .filter(|j| keys.contains(&format!("J:{}", j.id)))
            .cloned()
            .collect();
        if index == 0 {
            net.junctions = junctions;
            net.segments = segments;
        } else {
            islands.push((junctions, segments));
        }
    }
    islands
}

/// The standard post-removal consistency pass: drop floating nodes, split
/// disconnected islands into their own nets, re-derive logic, and collect
/// nets that lost all their segments.
fn repair_nets(
    wire_nets: &mut Vec<WireNet>,
    components: &[ProjectComponent],
    ctx: &KindCtx,
) -> Result<(), EditError> {
    for net in wire_nets.iter_mut() {
        drop_orphan_junctions(net);
    }
    split_all_nets(wire_nets);
    for net in wire_nets.iter_mut() {
        sync_net_logic(net, components, ctx)?;
    }
    gc_nets(wire_nets);
    Ok(())
}

/// A copyable snippet of a sheet.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub components: Vec<ProjectComponent>,
    pub notes: Vec<crate::project::Note>,
    pub wire_nets: Vec<WireNet>,
}

impl Fragment {
    pub fn is_empty(&self) -> bool {
        self.components.is_empty() && self.notes.is_empty()
    }

    /// Bounding box of the fragment's contents, in grid units.
    pub fn bounds(&self, abstractions: &[crate::project::ProjectAbstraction]) -> Option<Rect> {
        let mut bounds: Option<Rect> = None;
        let mut extend = |r: Rect| {
            bounds = Some(bounds.map_or(r, |b| b.union(&r)));
        };
        for c in &self.components {
            extend(layout::footprint(abstractions, c));
        }
        for n in &self.notes {
            extend(layout::note_bounds(n));
        }
        bounds
    }
}

/// A net exists as long as it has any wire in it. Stubs are legitimate;
/// only truly empty nets are collected.
fn gc_nets(wire_nets: &mut Vec<WireNet>) {
    wire_nets.retain(|n| !n.segments.is_empty());
}

// ── Normalization (repair pass for loaded / hand-edited files) ───────────────

/// Bring a project into the editor's invariants, repairing what can be
/// repaired. Returns human-readable notes about anything that was fixed.
/// Hand-written files without visual routing get port-to-port segments
/// synthesized so they render like any interactively drawn wire.
pub fn normalize(project: &mut Project) -> Vec<String> {
    let mut notes = Vec::new();
    if project.steps > 100_000 {
        notes.push(format!(
            "steps was {}, clamped to 100000",
            project.steps
        ));
        project.steps = 100_000;
    }
    let abstractions = project.abstractions.clone();

    let sheet_count = project.abstractions.len() + 1;
    for sheet_index in 0..sheet_count {
        let (label, interface_inputs, components, wire_nets): (String, Option<Vec<String>>, _, _) =
            if sheet_index == 0 {
                (
                    "main circuit".to_string(),
                    None,
                    &mut project.components,
                    &mut project.wire_nets,
                )
            } else {
                let a = &mut project.abstractions[sheet_index - 1];
                (
                    format!("abstraction \"{}\"", a.name),
                    Some(a.inputs.clone()),
                    &mut a.components,
                    &mut a.wire_nets,
                )
            };
        normalize_sheet(
            &abstractions,
            components,
            wire_nets,
            interface_inputs,
            &label,
            &mut notes,
        );
    }

    // Give legacy pins (from files predating free-floating pins) a stored
    // position so their layout stops depending on content bounds.
    for i in 0..project.abstractions.len() {
        let def = &project.abstractions[i];
        let mut missing: Vec<(String, PortKind, usize)> = Vec::new();
        for (n, name) in def.inputs.iter().enumerate() {
            if !def.pin_positions.contains_key(name) {
                missing.push((name.clone(), PortKind::Input, n));
            }
        }
        let mut outs: Vec<&String> = def.outputs.keys().collect();
        outs.sort();
        for (n, name) in outs.into_iter().enumerate() {
            if !def.pin_positions.contains_key(name) {
                missing.push((name.clone(), PortKind::Output, n));
            }
        }
        for (name, kind, n) in missing {
            let pos =
                layout::default_pin_position(&abstractions, &project.abstractions[i], kind, n);
            project.abstractions[i].pin_positions.insert(name, pos);
        }
    }

    // Output pin targets must reference an existing output port.
    for a in &mut project.abstractions {
        let comps = a.components.clone();
        for (pin, target) in a.outputs.iter_mut() {
            if target.is_empty() {
                continue; // unwired pin — legal while editing
            }
            let port = layout::output_target_port(target);
            let comp = comps.iter().find(|c| c.id == port.component_id);
            let ok = comp
                .and_then(|c| layout::type_ports(&abstractions, &c.component_type))
                .is_some_and(|ports| {
                    ports
                        .iter()
                        .any(|p| p.name == port.port_name && p.kind == PortKind::Output)
                });
            if !ok {
                notes.push(format!(
                    "abstraction \"{}\": output \"{pin}\" pointed at missing \"{target}\"; unwired it",
                    a.name
                ));
                target.clear();
                continue;
            }
            // The engine wants primitives bare and instance outputs dotted;
            // canonicalize hand-written variants like "gate.out" or "inst".
            let canonical = if crate::types::is_primitive(&comp.unwrap().component_type) {
                port.component_id.clone()
            } else {
                format!("{}.{}", port.component_id, port.port_name)
            };
            if *target != canonical {
                notes.push(format!(
                    "abstraction \"{}\": output \"{pin}\" target rewritten to \"{canonical}\"",
                    a.name
                ));
                *target = canonical;
            }
        }
    }
    notes
}

fn normalize_sheet(
    abstractions: &[crate::project::ProjectAbstraction],
    components: &mut [ProjectComponent],
    wire_nets: &mut Vec<WireNet>,
    interface_inputs: Option<Vec<String>>,
    label: &str,
    notes: &mut Vec<String>,
) {
    let port_kind = |port: &PortRef| -> Option<PortKind> {
        // INPUT:x pseudo-components (abstraction input pins) act as outputs,
        // but only for pins the abstraction actually declares.
        if let Some(name) = port.component_id.strip_prefix(crate::convert::INPUT_PREFIX) {
            return match &interface_inputs {
                Some(inputs) if inputs.iter().any(|i| i == name) => Some(PortKind::Output),
                _ => None,
            };
        }
        let comp = components.iter().find(|c| c.id == port.component_id)?;
        layout::type_ports(abstractions, &comp.component_type)?
            .into_iter()
            .find(|p| p.name == port.port_name)
            .map(|p| p.kind)
    };

    // 1. Drop logical references that don't resolve or have the wrong
    //    direction — they only seed segment synthesis below.
    for net in wire_nets.iter_mut() {
        if let Some(src) = &net.source {
            if port_kind(src) != Some(PortKind::Output) {
                notes.push(format!(
                    "{label}: wire \"{}\" had an invalid source ({}.{}); removed it",
                    net.id, src.component_id, src.port_name
                ));
                net.source = None;
            }
        }
        let net_id = net.id.clone();
        net.sinks.retain(|s| {
            let ok = port_kind(s) == Some(PortKind::Input);
            if !ok {
                notes.push(format!(
                    "{label}: wire \"{net_id}\" had an invalid sink ({}.{}); removed it",
                    s.component_id, s.port_name
                ));
            }
            ok
        });
    }

    // 2. Drop segments whose endpoints don't resolve (unknown components,
    //    ports, pins or junctions).
    for net in wire_nets.iter_mut() {
        let before = net.segments.len();
        let junction_ids: Vec<String> = net.junctions.iter().map(|j| j.id.clone()).collect();
        net.segments.retain(|seg| {
            [&seg.from, &seg.to].iter().all(|e| match e {
                Endpoint::Port {
                    component_id,
                    port_name,
                } => port_kind(&PortRef {
                    component_id: component_id.clone(),
                    port_name: port_name.clone(),
                })
                .is_some(),
                Endpoint::Junction { junction_id } => junction_ids.contains(junction_id),
            })
        });
        if net.segments.len() != before {
            notes.push(format!(
                "{label}: wire \"{}\" had dangling segments; removed them",
                net.id
            ));
        }
    }

    // 3. Synthesize wires for logical-only files: a stored sink with no
    //    segment gets a direct source→sink segment.
    for net in wire_nets.iter_mut() {
        let Some(source) = net.source.clone() else {
            continue;
        };
        for sink in net.sinks.clone() {
            let has_path = net.segments.iter().any(|seg| {
                [&seg.from, &seg.to]
                    .iter()
                    .any(|e| endpoint_is_port(e, &sink))
            });
            if !has_path {
                net.segments.push(crate::project::Segment {
                    from: port_endpoint(&source),
                    to: port_endpoint(&sink),
                });
            }
        }
    }

    // 4. An input is wired at most once, globally: the first segment wins.
    // Decide keep/drop per segment first, then register its inputs — so a
    // dropped segment can't poison a later, legitimate connection.
    let mut seen_inputs: Vec<PortRef> = Vec::new();
    for net in wire_nets.iter_mut() {
        let net_id = net.id.clone();
        net.segments.retain(|seg| {
            let mut segment_inputs: Vec<PortRef> = Vec::new();
            for e in [&seg.from, &seg.to] {
                let Endpoint::Port {
                    component_id,
                    port_name,
                } = e
                else {
                    continue;
                };
                let p = PortRef {
                    component_id: component_id.clone(),
                    port_name: port_name.clone(),
                };
                if port_kind(&p) == Some(PortKind::Input) {
                    if seen_inputs.contains(&p) || segment_inputs.contains(&p) {
                        notes.push(format!(
                            "{label}: input {}.{} was wired twice; kept the first (removed from \"{net_id}\")",
                            p.component_id, p.port_name
                        ));
                        return false;
                    }
                    segment_inputs.push(p);
                }
            }
            seen_inputs.extend(segment_inputs);
            true
        });
    }

    // 5. One driving output per net: segments touching a second, different
    //    output are removed.
    for net in wire_nets.iter_mut() {
        let mut first_output: Option<PortRef> = None;
        let net_id = net.id.clone();
        net.segments.retain(|seg| {
            for e in [&seg.from, &seg.to] {
                let Endpoint::Port {
                    component_id,
                    port_name,
                } = e
                else {
                    continue;
                };
                let p = PortRef {
                    component_id: component_id.clone(),
                    port_name: port_name.clone(),
                };
                if port_kind(&p) == Some(PortKind::Output) {
                    match &first_output {
                        None => first_output = Some(p),
                        Some(f) if *f == p => {}
                        Some(f) => {
                            notes.push(format!(
                                "{label}: wire \"{net_id}\" was driven by both {}.{} and {}.{}; kept the first",
                                f.component_id, f.port_name, p.component_id, p.port_name
                            ));
                            return false;
                        }
                    }
                }
            }
            true
        });
    }

    // 6. Nets sharing a driving source are one electrical net (connected
    //    through the shared port): merge them.
    let derived_source = |net: &WireNet| -> Option<PortRef> {
        net.segments.iter().find_map(|seg| {
            [&seg.from, &seg.to].iter().find_map(|e| {
                let Endpoint::Port {
                    component_id,
                    port_name,
                } = e
                else {
                    return None;
                };
                let p = PortRef {
                    component_id: component_id.clone(),
                    port_name: port_name.clone(),
                };
                (port_kind(&p) == Some(PortKind::Output)).then_some(p)
            })
        })
    };
    let mut i = 0;
    while i < wire_nets.len() {
        let Some(source) = derived_source(&wire_nets[i]) else {
            i += 1;
            continue;
        };
        let mut j = i + 1;
        while j < wire_nets.len() {
            if derived_source(&wire_nets[j]) == Some(source.clone()) {
                let absorbed = wire_nets.remove(j);
                notes.push(format!(
                    "{label}: merged wire \"{}\" into \"{}\" (same source)",
                    absorbed.id, wire_nets[i].id
                ));
                merge_net(&mut wire_nets[i], absorbed);
            } else {
                j += 1;
            }
        }
        i += 1;
    }

    // 7. Standard graph repair: floating nodes, disconnected islands,
    //    derived logic, empty nets.
    for net in wire_nets.iter_mut() {
        drop_orphan_junctions(net);
    }
    split_all_nets(wire_nets);
    let ctx = KindCtx {
        abstractions: abstractions.to_vec(),
        interface_outputs: interface_inputs.as_ref().map(|_| vec![]),
        interface_inputs,
    };
    for net in wire_nets.iter_mut() {
        if let Err(e) = sync_net_logic(net, components, &ctx) {
            notes.push(format!(
                "{label}: wire \"{}\" could not be repaired ({e}); removed it",
                net.id
            ));
            net.segments.clear();
        }
    }
    let before = wire_nets.len();
    gc_nets(wire_nets);
    if wire_nets.len() != before {
        notes.push(format!(
            "{label}: removed {} wire(s) that connected nothing",
            before - wire_nets.len()
        ));
    }

    // Duplicate component IDs can't be auto-repaired (we can't know which
    // wires meant which component) — report them; compile will refuse too.
    let mut seen = Vec::new();
    for c in components.iter() {
        if seen.contains(&&c.id) {
            notes.push(format!(
                "{label}: duplicate component id \"{}\" — fix this in the JSON",
                c.id
            ));
        }
        seen.push(&c.id);
    }
}

/// Fold `absorbed` into `net`, renaming colliding junction IDs.
/// Returns the (old, new) id pairs of renamed junctions.
fn merge_net(net: &mut WireNet, mut absorbed: WireNet) -> Vec<(String, String)> {
    let mut renames = Vec::new();
    for junction in &mut absorbed.junctions {
        if net.junctions.iter().any(|j| j.id == junction.id) {
            let new_id = generate_junction_id(net);
            for seg in &mut absorbed.segments {
                for e in [&mut seg.from, &mut seg.to] {
                    if matches!(e, Endpoint::Junction { junction_id } if *junction_id == junction.id)
                    {
                        *e = Endpoint::Junction {
                            junction_id: new_id.clone(),
                        };
                    }
                }
            }
            renames.push((junction.id.clone(), new_id.clone()));
            junction.id = new_id;
        }
        net.junctions.push(junction.clone());
    }
    if net.source.is_none() {
        net.source = absorbed.source.take();
    }
    net.sinks.append(&mut absorbed.sinks);
    net.segments.append(&mut absorbed.segments);
    renames
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectAbstraction;

    fn port(c: &str, p: &str) -> PortRef {
        PortRef {
            component_id: c.into(),
            port_name: p.into(),
        }
    }

    fn editor_with_two_nands() -> Editor {
        let mut e = Editor::empty();
        e.place_component("nand", [0, 0]).unwrap();
        e.place_component("nand", [10, 0]).unwrap();
        e
    }

    #[test]
    fn place_generates_sequential_ids() {
        let mut e = Editor::empty();
        assert_eq!(e.place_component("nand", [0, 0]).unwrap(), "nand_1");
        assert_eq!(e.place_component("nand", [10, 0]).unwrap(), "nand_2");
        assert_eq!(e.place_component("dff", [20, 0]).unwrap(), "dff_1");
        // dff gets a default init so the checkbox has a value.
        assert_eq!(e.project.components[2].init, Some(false));
    }

    #[test]
    fn place_rejects_overlap_and_unknown_type() {
        let mut e = Editor::empty();
        e.place_component("nand", [0, 0]).unwrap();
        assert_eq!(
            e.place_component("nand", [2, 1]).unwrap_err(),
            EditError::Overlap
        );
        // Touching edge-to-edge is allowed.
        e.place_component("nand", [3, 0]).unwrap();
        assert!(matches!(
            e.place_component("bogus", [20, 20]).unwrap_err(),
            EditError::UnknownComponentType(_)
        ));
        // Failed ops must not leave undo steps or mutations.
        assert_eq!(e.project.components.len(), 2);
    }

    #[test]
    fn clocks_are_a_shared_signal() {
        // Several clk components — even inside abstractions — are all the
        // same clock. Placing them anywhere is legal and they tick together.
        let mut e = Editor::empty();
        e.place_component("clk", [0, 0]).unwrap();
        e.place_component("clk", [10, 0]).unwrap();
        let i = e.create_abstraction("ticker").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        e.place_component("clk", [0, 0]).unwrap();
        e.place_component("dff", [6, 0]).unwrap();
        e.connect(port("clk_1", "out"), port("dff_1", "clk"), &[]).unwrap();
        e.connect(port("dff_1", "out"), port("dff_1", "d"), &[]).unwrap();
        e.connect(port("dff_1", "out"), pin("OUTPUT:", "q"), &[]).unwrap();
        e.open_sheet(SheetRef::Top).unwrap();
        e.place_component("ticker", [0, 8]).unwrap();

        use crate::compiler::compile;
        use crate::convert::project_to_circuit;
        use crate::expander::expand;
        use crate::simulator::simulate;
        let expanded = expand(&project_to_circuit(&e.project).unwrap()).unwrap();
        let compiled = compile(&expanded.def).unwrap();
        let results = simulate(&compiled);
        // Both top-level clocks and the one inside the abstraction agree.
        for (step, state) in results.iter().enumerate() {
            assert_eq!(state["clk_1"], state["clk_2"]);
            assert_eq!(state["clk_1"], step % 2 == 1);
        }
    }

    #[test]
    fn connect_in_either_order_and_replace_driver() {
        let mut e = editor_with_two_nands();
        // output → input
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
            .unwrap();
        // input → output (reversed argument order must also work)
        e.connect(port("nand_2", "b"), port("nand_1", "out"), &[])
            .unwrap();
        assert_eq!(e.project.wire_nets.len(), 1, "fan-out merges into one net");
        assert_eq!(e.project.wire_nets[0].sinks.len(), 2);

        // A third component takes over nand_2.a: old connection replaced.
        e.place_component("clk", [0, 10]).unwrap();
        e.connect(port("clk_1", "out"), port("nand_2", "a"), &[])
            .unwrap();
        let driving_a: Vec<_> = e
            .project
            .wire_nets
            .iter()
            .filter(|n| n.sinks.contains(&port("nand_2", "a")))
            .collect();
        assert_eq!(driving_a.len(), 1);
        assert_eq!(driving_a[0].source, Some(port("clk_1", "out")));
    }

    #[test]
    fn connect_rejects_same_kind() {
        let mut e = editor_with_two_nands();
        assert_eq!(
            e.connect(port("nand_1", "out"), port("nand_2", "out"), &[])
                .unwrap_err(),
            EditError::NotAnInputAndOutput
        );
        assert_eq!(
            e.connect(port("nand_1", "a"), port("nand_2", "b"), &[])
                .unwrap_err(),
            EditError::NotAnInputAndOutput
        );
        assert!(e.project.wire_nets.is_empty());
    }

    #[test]
    fn connect_with_bends_creates_junction_chain() {
        let mut e = editor_with_two_nands();
        e.connect(
            port("nand_1", "out"),
            port("nand_2", "a"),
            &[[5.0, 5.0], [7.0, 5.0]],
        )
        .unwrap();
        let net = &e.project.wire_nets[0];
        assert_eq!(net.junctions.len(), 2);
        assert_eq!(net.segments.len(), 3);
        // Path is connected: port → j1 → j2 → port.
        assert!(matches!(net.segments[0].from, Endpoint::Port { .. }));
        assert!(matches!(net.segments[2].to, Endpoint::Port { .. }));
    }

    #[test]
    fn bends_reverse_when_drawn_from_input() {
        let mut e = editor_with_two_nands();
        // Drawn starting at the input, bending down then right.
        e.connect(
            port("nand_2", "a"),
            port("nand_1", "out"),
            &[[8.0, 5.0], [4.0, 5.0]],
        )
        .unwrap();
        let net = &e.project.wire_nets[0];
        // First junction in the stored path must be the one nearest the
        // SOURCE (bends reversed): [4,5] before [8,5].
        assert_eq!(net.junctions[0].pos, [4.0, 5.0]);
    }

    #[test]
    fn delete_component_keeps_wire_stub() {
        let mut e = editor_with_two_nands();
        e.place_component("clk", [0, 10]).unwrap();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 1.0]])
            .unwrap();
        e.connect(port("nand_1", "out"), port("nand_2", "b"), &[])
            .unwrap();
        e.delete_components(&["nand_2".into()]).unwrap();
        assert_eq!(e.project.components.len(), 2);
        // Wires are placed objects: the source→node stub survives the
        // deletion of the component it pointed at.
        assert_eq!(e.project.wire_nets.len(), 1);
        let net = &e.project.wire_nets[0];
        assert_eq!(net.segments.len(), 1);
        assert_eq!(net.junctions.len(), 1);
        assert_eq!(net.source, Some(port("nand_1", "out")));
        assert!(net.sinks.is_empty());
    }

    #[test]
    fn delete_source_component_keeps_nothing_dangling() {
        let mut e = editor_with_two_nands();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
            .unwrap();
        e.delete_components(&["nand_1".into()]).unwrap();
        // Sink-only net without a source connects nothing visual anymore:
        // the segment referenced the deleted port, so it's gone; and with
        // no source the net is pure junk → collected.
        assert!(e.project.wire_nets.is_empty());
    }

    #[test]
    fn disconnect_single_sink_keeps_other_branch() {
        let mut e = editor_with_two_nands();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 0.0]])
            .unwrap();
        e.connect(port("nand_1", "out"), port("nand_2", "b"), &[[5.0, 3.0]])
            .unwrap();
        e.disconnect_sink(port("nand_2", "a")).unwrap();
        let net = &e.project.wire_nets[0];
        assert_eq!(net.sinks, vec![port("nand_2", "b")]);
        // Both nodes survive — the a-branch keeps its node as a stub.
        assert_eq!(net.junctions.len(), 2);
        // Disconnecting the other sink leaves a sourced net of stubs.
        e.disconnect_sink(port("nand_2", "b")).unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);
        assert!(e.project.wire_nets[0].sinks.is_empty());
        assert_eq!(
            e.project.wire_nets[0].source,
            Some(port("nand_1", "out"))
        );
    }

    #[test]
    fn move_rejects_overlap_but_allows_group_moves() {
        let mut e = editor_with_two_nands(); // at x=0 and x=10
        assert_eq!(
            e.move_components(&["nand_1".into()], [9, 0]).unwrap_err(),
            EditError::Overlap
        );
        // Moving both together over each other's old spots is fine.
        e.move_components(&["nand_1".into(), "nand_2".into()], [10, 0])
            .unwrap();
        assert_eq!(e.project.components[0].pos, [10.0, 0.0]);
        assert_eq!(e.project.components[1].pos, [20.0, 0.0]);
    }

    #[test]
    fn zero_move_is_not_an_undo_step() {
        let mut e = editor_with_two_nands();
        let undo_before = e.undo_stack.len();
        e.move_components(&["nand_1".into()], [0, 0]).unwrap();
        assert_eq!(e.undo_stack.len(), undo_before);
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut e = Editor::empty();
        e.place_component("nand", [0, 0]).unwrap();
        e.place_component("dff", [10, 0]).unwrap();
        e.connect(port("nand_1", "out"), port("dff_1", "d"), &[])
            .unwrap();

        e.undo().unwrap(); // un-connect
        assert!(e.project.wire_nets.is_empty());
        e.undo().unwrap(); // un-place dff
        assert_eq!(e.project.components.len(), 1);
        e.redo().unwrap();
        e.redo().unwrap();
        assert_eq!(e.project.components.len(), 2);
        assert_eq!(e.project.wire_nets.len(), 1);
        assert_eq!(e.redo().unwrap_err(), EditError::NothingToRedo);

        // A new edit clears the redo tail.
        e.undo().unwrap();
        e.place_component("clk", [20, 0]).unwrap();
        assert_eq!(e.redo().unwrap_err(), EditError::NothingToRedo);
    }

    #[test]
    fn failed_op_is_not_undoable_and_rolls_back() {
        let mut e = editor_with_two_nands();
        let snapshot = e.project.clone();
        let _ = e.connect(port("nand_1", "out"), port("nand_2", "out"), &[]);
        assert_eq!(
            serde_json::to_string(&e.project).unwrap(),
            serde_json::to_string(&snapshot).unwrap()
        );
        // Two undos available (the two placements), not three.
        e.undo().unwrap();
        e.undo().unwrap();
        assert_eq!(e.undo().unwrap_err(), EditError::NothingToUndo);
    }

    #[test]
    fn toggle_init() {
        let mut e = Editor::empty();
        e.place_component("dff", [0, 0]).unwrap();
        assert!(e.toggle_dff_init("dff_1").unwrap());
        assert!(!e.toggle_dff_init("dff_1").unwrap());
        e.place_component("nand", [10, 0]).unwrap();
        assert!(matches!(
            e.toggle_dff_init("nand_1").unwrap_err(),
            EditError::NotADff(_)
        ));
    }

    #[test]
    fn self_connection_on_dff_is_allowed() {
        let mut e = Editor::empty();
        e.place_component("dff", [0, 0]).unwrap();
        e.connect(port("dff_1", "out"), port("dff_1", "d"), &[])
            .unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);
    }

    // ── Abstraction sheets ───────────────────────────────────────────────

    fn project_with_not_abstraction() -> Editor {
        let mut e = Editor::empty();
        e.project.abstractions.push(ProjectAbstraction {
            pin_positions: HashMap::new(),
            port_layout: HashMap::new(),
            name: "not".into(),
            inputs: vec!["x".into()],
            outputs: HashMap::from([("out".into(), "gate".into())]),
            width: 4.0,
            height: 2.0,
            components: vec![ProjectComponent {
                id: "gate".into(),
                component_type: "nand".into(),
                init: None,
                pos: [2.0, 2.0],
            }],
            wire_nets: vec![],
        notes: vec![],
        });
        e
    }

    #[test]
    fn no_recursive_abstractions() {
        let mut e = project_with_not_abstraction();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        assert!(matches!(
            e.place_component("not", [8, 0]).unwrap_err(),
            EditError::RecursiveAbstraction(_)
        ));
        // But placing it at top level is fine.
        e.open_sheet(SheetRef::Top).unwrap();
        e.place_component("not", [0, 0]).unwrap();
    }

    #[test]
    fn undo_restores_sheet() {
        let mut e = project_with_not_abstraction();
        e.place_component("nand", [0, 10]).unwrap();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        e.place_component("nand", [8, 0]).unwrap();
        e.undo().unwrap();
        assert_eq!(e.sheet, SheetRef::Abstraction(0));
        e.undo().unwrap();
        assert_eq!(e.sheet, SheetRef::Top);
    }

    // ── Wire graph (nodes as first-class objects) ────────────────────────

    fn node(net_id: &str, junction_id: &str) -> WirePoint {
        WirePoint::Junction {
            net_id: net_id.into(),
            junction_id: junction_id.into(),
        }
    }

    #[test]
    fn add_wire_incrementally_and_branch_from_node() {
        let mut e = editor_with_two_nands();
        // Out → node: a committed stub.
        let a = e
            .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 1.0]))
            .unwrap();
        let n1 = a.new_junction.unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);
        assert_eq!(e.project.wire_nets[0].sinks.len(), 0);
        assert_eq!(e.project.wire_nets[0].source, Some(port("nand_1", "out")));

        // Node → input finishes the connection.
        e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "a")))
            .unwrap();
        assert_eq!(e.project.wire_nets[0].sinks, vec![port("nand_2", "a")]);

        // Branch from the SAME node to the other input.
        e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "b")))
            .unwrap();
        assert_eq!(e.project.wire_nets[0].sinks.len(), 2);
        assert_eq!(e.project.wire_nets[0].segments.len(), 3);

        // Duplicate wire between the same points is a no-op.
        assert_eq!(
            e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "b")))
                .unwrap_err(),
            EditError::NoopWire
        );
    }

    #[test]
    fn two_outputs_cannot_drive_one_wire() {
        let mut e = editor_with_two_nands();
        let a = e
            .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 5.0]))
            .unwrap();
        let n = a.new_junction.unwrap();
        assert!(matches!(
            e.add_wire(node(&a.net_id, &n), WirePoint::Port(port("nand_2", "out")))
                .unwrap_err(),
            EditError::TwoSources(_)
        ));
        // Rolled back: still one segment, and nand_2.out is untouched.
        assert_eq!(e.project.wire_nets[0].segments.len(), 1);
    }

    #[test]
    fn merging_two_stub_nets_via_wire() {
        let mut e = editor_with_two_nands();
        // A sourced stub and a sunk stub.
        let a = e
            .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 0.0]))
            .unwrap();
        let b = e
            .add_wire(WirePoint::Port(port("nand_2", "a")), WirePoint::New([7.0, 3.0]))
            .unwrap();
        assert_eq!(e.project.wire_nets.len(), 2);
        // Wire node-to-node: nets merge and the connection goes live.
        e.add_wire(
            node(&a.net_id, &a.new_junction.clone().unwrap()),
            node(&b.net_id, &b.new_junction.clone().unwrap()),
        )
        .unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);
        let net = &e.project.wire_nets[0];
        assert_eq!(net.source, Some(port("nand_1", "out")));
        assert_eq!(net.sinks, vec![port("nand_2", "a")]);
        assert_eq!(net.segments.len(), 3);
    }

    #[test]
    fn tied_inputs_get_driven_when_source_arrives() {
        let mut e = editor_with_two_nands();
        // Input-to-input wire: legal, a sourceless net with two sinks.
        e.add_wire(
            WirePoint::Port(port("nand_2", "a")),
            WirePoint::Port(port("nand_2", "b")),
        )
        .unwrap();
        let net = &e.project.wire_nets[0];
        assert_eq!(net.source, None);
        assert_eq!(net.sinks.len(), 2);
        // Attaching an output drives both.
        e.add_wire(
            WirePoint::Port(port("nand_1", "out")),
            WirePoint::Port(port("nand_2", "a")),
        )
        .unwrap();
        // The replace rule detached a first... so instead wire to a node of
        // that net: rebuild the scenario through a node.
        let mut e = editor_with_two_nands();
        let a = e
            .add_wire(WirePoint::Port(port("nand_2", "a")), WirePoint::New([5.0, 1.0]))
            .unwrap();
        let n = a.new_junction.unwrap();
        e.add_wire(node(&a.net_id, &n), WirePoint::Port(port("nand_2", "b")))
            .unwrap();
        e.add_wire(WirePoint::Port(port("nand_1", "out")), node(&a.net_id, &n))
            .unwrap();
        let net = &e.project.wire_nets[0];
        assert_eq!(net.source, Some(port("nand_1", "out")));
        assert_eq!(net.sinks.len(), 2);
    }

    #[test]
    fn deleting_a_node_splits_the_net() {
        let mut e = editor_with_two_nands();
        // out → n1 → n2 → a  (a chain with two nodes)
        let a = e
            .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([4.0, 4.0]))
            .unwrap();
        let n1 = a.new_junction.unwrap();
        let b = e
            .add_wire(node(&a.net_id, &n1), WirePoint::New([8.0, 4.0]))
            .unwrap();
        let n2 = b.new_junction.unwrap();
        e.add_wire(node(&b.net_id, &n2), WirePoint::Port(port("nand_2", "a")))
            .unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);

        // Branch a second island off n2, then delete n2: the pieces that
        // met there fall apart into separate nets.
        let c = e
            .add_wire(node(&b.net_id, &n2), WirePoint::New([8.0, 8.0]))
            .unwrap();
        let n3 = c.new_junction.unwrap();
        e.delete_junction(&c.net_id, &n2).unwrap();

        // Remaining: out→n1 (sourced stub), and n3 alone died with its only
        // segment... n3's segment touched n2 so it's gone; nand_2.a's segment
        // also touched n2 and is gone.
        assert_eq!(e.project.wire_nets.len(), 1);
        let net = &e.project.wire_nets[0];
        assert_eq!(net.source, Some(port("nand_1", "out")));
        assert!(net.sinks.is_empty());
        assert!(net.junctions.iter().any(|j| j.id == n1));
        assert!(!net.junctions.iter().any(|j| j.id == n3));

        // Undo restores the whole arrangement.
        e.undo().unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);
        assert_eq!(e.project.wire_nets[0].junctions.len(), 3);
        assert_eq!(e.project.wire_nets[0].sinks, vec![port("nand_2", "a")]);
    }

    #[test]
    fn splitting_preserves_both_live_halves() {
        let mut e = editor_with_two_nands();
        e.place_component("clk", [0, 10]).unwrap();
        // clk.out → n1 → n2, n2 → nand_2.a, and n1 → nand_2.b:
        let a = e
            .add_wire(WirePoint::Port(port("clk_1", "out")), WirePoint::New([4.0, 8.0]))
            .unwrap();
        let n1 = a.new_junction.unwrap();
        let b = e
            .add_wire(node(&a.net_id, &n1), WirePoint::New([8.0, 8.0]))
            .unwrap();
        let n2 = b.new_junction.unwrap();
        e.add_wire(node(&b.net_id, &n2), WirePoint::Port(port("nand_2", "a")))
            .unwrap();
        e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "b")))
            .unwrap();

        // Deleting n1 cuts clk and the b-branch off; n2→a survives as its
        // own (sourceless) net.
        e.delete_junction(&a.net_id, &n1).unwrap();
        assert_eq!(e.project.wire_nets.len(), 1);
        let net = &e.project.wire_nets[0];
        assert_eq!(net.source, None);
        assert_eq!(net.sinks, vec![port("nand_2", "a")]);
        assert_eq!(net.junctions.len(), 1);
    }

    #[test]
    fn move_junction_updates_position() {
        let mut e = editor_with_two_nands();
        let a = e
            .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 1.0]))
            .unwrap();
        let n = a.new_junction.unwrap();
        let undo_before = e.undo_stack.len();
        e.move_junction(&a.net_id, &n, [5.0, 1.0]).unwrap(); // no-op
        assert_eq!(e.undo_stack.len(), undo_before);
        e.move_junction(&a.net_id, &n, [9.0, 4.5]).unwrap();
        assert_eq!(e.project.wire_nets[0].junctions[0].pos, [9.0, 4.5]);
        assert!(matches!(
            e.move_junction(&a.net_id, "ghost", [0.0, 0.0]).unwrap_err(),
            EditError::UnknownJunction(_)
        ));
    }

    #[test]
    fn output_pin_wired_from_a_node() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("buf").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "b"), &[]).unwrap();

        // Drive a node from the gate, then wire the node into the pin.
        let a = e
            .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([9.0, 1.0]))
            .unwrap();
        let n = a.new_junction.unwrap();
        e.add_wire(node(&a.net_id, &n), WirePoint::Port(pin("OUTPUT:", "q")))
            .unwrap();
        assert_eq!(
            e.project.abstractions[i].outputs.get("q"),
            Some(&"nand_1".to_string())
        );

        // A sourceless node can't feed an output pin.
        let b = e
            .add_wire(WirePoint::Port(port("nand_1", "a")), WirePoint::New([2.0, 6.0]))
            .unwrap();
        // (nand_1.a was replaced-off the INPUT:x net onto a fresh stub.)
        let m = b.new_junction.unwrap();
        assert_eq!(
            e.add_wire(node(&b.net_id, &m), WirePoint::Port(pin("OUTPUT:", "q")))
                .unwrap_err(),
            EditError::OutputPinNeedsSource
        );
    }

    // ── Interface pins ───────────────────────────────────────────────────

    fn pin(kind_prefix: &str, name: &str) -> PortRef {
        PortRef {
            component_id: format!("{kind_prefix}{name}"),
            port_name: name.into(),
        }
    }

    #[test]
    fn create_abstraction_and_edit_interface() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("adder").unwrap();
        assert!(matches!(
            e.create_abstraction("adder").unwrap_err(),
            EditError::NameTaken(_)
        ));
        assert!(matches!(
            e.create_abstraction("nand").unwrap_err(),
            EditError::NameTaken(_)
        ));
        assert!(matches!(
            e.create_abstraction("has space").unwrap_err(),
            EditError::InvalidName(_)
        ));

        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "a").unwrap();
        e.add_interface_pin(PortKind::Input, "b").unwrap();
        e.add_interface_pin(PortKind::Output, "sum").unwrap();
        assert!(matches!(
            e.add_interface_pin(PortKind::Output, "a").unwrap_err(),
            EditError::NameTaken(_)
        ));
        let def = &e.project.abstractions[i];
        assert_eq!(def.inputs, vec!["a", "b"]);
        assert_eq!(def.outputs.get("sum"), Some(&String::new()));
    }

    #[test]
    fn wire_input_pin_through_gate_to_output_pin() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("buf2").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.add_interface_pin(PortKind::Output, "out").unwrap();
        e.place_component("nand", [4, 0]).unwrap();

        // Input pin drives both nand inputs (drawn from pin or port side).
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
        e.connect(port("nand_1", "b"), pin("INPUT:", "x"), &[]).unwrap();
        let nets = &e.project.abstractions[i].wire_nets;
        assert_eq!(nets.len(), 1, "one net per source, even for pins");
        assert_eq!(nets[0].sinks.len(), 2);

        // Gate output feeds the output pin: recorded in the outputs map.
        e.connect(port("nand_1", "out"), pin("OUTPUT:", "out"), &[])
            .unwrap();
        assert_eq!(
            e.project.abstractions[i].outputs.get("out"),
            Some(&"nand_1".to_string())
        );

        // Direct input→output passthrough is rejected.
        assert_eq!(
            e.connect(pin("INPUT:", "x"), pin("OUTPUT:", "out"), &[])
                .unwrap_err(),
            EditError::PassthroughNotAllowed
        );

        // The finished abstraction actually works end to end.
        e.open_sheet(SheetRef::Top).unwrap();
        e.place_component("dff", [0, 0]).unwrap();
        e.toggle_dff_init("dff_1").unwrap();
        e.place_component("buf2", [8, 0]).unwrap();
        e.connect(port("dff_1", "out"), port("buf2_1", "x"), &[])
            .unwrap();
        let session_result = {
            use crate::compiler::compile;
            use crate::convert::project_to_circuit;
            use crate::expander::expand;
            use crate::simulator::simulate;
            let def = project_to_circuit(&e.project).unwrap();
            let expanded = expand(&def).unwrap();
            let compiled = compile(&expanded.def).unwrap();
            let mut results = simulate(&compiled);
            for step in &mut results {
                for (alias, target) in &expanded.aliases {
                    if let Some(&v) = step.get(target) {
                        step.insert(alias.clone(), v);
                    }
                }
            }
            results
        };
        // buf2 is NOT(NOT... actually a single nand with both inputs = x,
        // so out = NAND(1,1) = 0.
        assert!(!session_result[0]["buf2_1.out"]);
    }

    #[test]
    fn remove_input_pin_scrubs_wires() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();

        e.remove_interface_pin(PortKind::Input, "x").unwrap();
        let def = &e.project.abstractions[i];
        assert!(def.inputs.is_empty());
        assert!(def.wire_nets.is_empty(), "pin's net is gone");
        // Undo brings both back.
        e.undo().unwrap();
        let def = &e.project.abstractions[i];
        assert_eq!(def.inputs, vec!["x"]);
        assert_eq!(def.wire_nets.len(), 1);
    }

    #[test]
    fn deleting_component_unwires_output_pin() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[])
            .unwrap();
        e.delete_components(&["nand_1".into()]).unwrap();
        assert_eq!(
            e.project.abstractions[i].outputs.get("q"),
            Some(&String::new())
        );
    }

    #[test]
    fn normalize_rejects_undeclared_input_pins() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();

        // A hand-edited file with a corrupted stored source but an intact
        // segment graph heals: the source is re-derived from the segments.
        let mut project = e.project.clone();
        project.abstractions[i].wire_nets[0].source = Some(pin("INPUT:", "ghost"));
        let notes = normalize(&mut project);
        assert_eq!(
            project.abstractions[i].wire_nets[0].source,
            Some(pin("INPUT:", "x"))
        );
        assert!(!notes.is_empty());

        // But segments referencing an undeclared pin are truly dangling.
        let mut project = e.project.clone();
        for net in &mut project.abstractions[i].wire_nets {
            net.source = Some(pin("INPUT:", "ghost"));
            for seg in &mut net.segments {
                for ep in [&mut seg.from, &mut seg.to] {
                    if matches!(ep, Endpoint::Port { component_id, .. }
                        if component_id.starts_with("INPUT:"))
                    {
                        *ep = Endpoint::Port {
                            component_id: "INPUT:ghost".into(),
                            port_name: "ghost".into(),
                        };
                    }
                }
            }
        }
        let notes = normalize(&mut project);
        assert!(project.abstractions[i].wire_nets.is_empty());
        assert!(!notes.is_empty());

        // And an outputs map pointing at nothing gets unwired with a note.
        let mut project = e.project.clone();
        project.abstractions[i]
            .outputs
            .insert("q".into(), "ghost.out".into());
        let notes = normalize(&mut project);
        assert_eq!(project.abstractions[i].outputs["q"], "");
        assert!(!notes.is_empty());
    }

    #[test]
    fn const_component_drives_signals() {
        let mut e = Editor::empty();
        e.place_component("const", [0, 0]).unwrap();
        assert!(e.toggle_dff_init("const_1").unwrap());
        e.place_component("nand", [6, 0]).unwrap();
        e.connect(port("const_1", "out"), port("nand_1", "a"), &[])
            .unwrap();
        // nand.b stays unconnected: reads as 0 → NAND(1, 0) = 1.
        use crate::compiler::compile;
        use crate::convert::project_to_circuit;
        use crate::expander::expand;
        use crate::simulator::simulate;
        let expanded = expand(&project_to_circuit(&e.project).unwrap()).unwrap();
        let compiled = compile(&expanded.def).unwrap();
        let results = simulate(&compiled);
        assert!(results[0]["const_1"]);
        assert!(results[0]["nand_1"]);
    }

    // ── Notes ────────────────────────────────────────────────────────────

    #[test]
    fn notes_place_edit_move_delete() {
        let mut e = Editor::empty();
        let id = e.place_note([2.0, 3.0], "reset logic lives here").unwrap();
        assert_eq!(id, "note_1");
        e.move_items(&[], std::slice::from_ref(&id), [4, 1]).unwrap();
        assert_eq!(e.project.notes[0].pos, [6.0, 4.0]);
        e.set_note_text(&id, "updated").unwrap();
        assert_eq!(e.project.notes[0].text, "updated");
        // Empty text deletes.
        e.set_note_text(&id, "  ").unwrap();
        assert!(e.project.notes.is_empty());
        e.undo().unwrap();
        assert_eq!(e.project.notes[0].text, "updated");
    }

    #[test]
    fn notes_delete_together_with_components() {
        let mut e = Editor::empty();
        e.place_component("nand", [0, 0]).unwrap();
        let note = e.place_note([5.0, 5.0], "hello").unwrap();
        e.delete_items(&["nand_1".into()], &[note]).unwrap();
        assert!(e.project.components.is_empty());
        assert!(e.project.notes.is_empty());
        // One undo restores both (single op).
        e.undo().unwrap();
        assert_eq!(e.project.components.len(), 1);
        assert_eq!(e.project.notes.len(), 1);
    }

    // ── Renames & abstraction deletion ───────────────────────────────────

    fn abstraction_with_instance() -> Editor {
        let mut e = Editor::empty();
        let i = e.create_abstraction("buf").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "b"), &[]).unwrap();
        e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[])
            .unwrap();
        e.open_sheet(SheetRef::Top).unwrap();
        e.place_component("dff", [0, 0]).unwrap();
        e.place_component("buf", [8, 0]).unwrap();
        // IDs are unique per sheet, so this is nand_1 at top level.
        let top_nand = e.place_component("nand", [16, 0]).unwrap();
        e.connect(port("dff_1", "out"), port("buf_1", "x"), &[]).unwrap();
        e.connect(port("buf_1", "q"), port(&top_nand, "a"), &[]).unwrap();
        e
    }

    #[test]
    fn rename_input_pin_rewrites_everything() {
        let mut e = abstraction_with_instance();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        e.rename_interface_pin(PortKind::Input, "x", "data").unwrap();

        let def = &e.project.abstractions[0];
        assert_eq!(def.inputs, vec!["data"]);
        // Inside: nets now reference INPUT:data.
        assert_eq!(
            def.wire_nets[0].source,
            Some(pin("INPUT:", "data"))
        );
        // Outside: the instance's wired port follows.
        assert!(e
            .project
            .wire_nets
            .iter()
            .any(|n| n.sinks.contains(&port("buf_1", "data"))));
        // The circuit still compiles and runs.
        use crate::convert::project_to_circuit;
        assert!(project_to_circuit(&e.project).is_ok());
        let mut p = e.project.clone();
        assert!(normalize(&mut p).is_empty(), "rename left inconsistencies");
    }

    #[test]
    fn rename_output_pin_rewrites_instance_wires_and_dot_targets() {
        let mut e = abstraction_with_instance();
        // Nest buf inside a second abstraction that exports its output.
        let j = e.create_abstraction("wrapper").unwrap();
        e.open_sheet(SheetRef::Abstraction(j)).unwrap();
        e.add_interface_pin(PortKind::Output, "w").unwrap();
        e.place_component("buf", [4, 0]).unwrap();
        e.connect(port("buf_1", "q"), pin("OUTPUT:", "w"), &[]).unwrap();
        assert_eq!(
            e.project.abstractions[j].outputs.get("w"),
            Some(&"buf_1.q".to_string())
        );

        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        e.rename_interface_pin(PortKind::Output, "q", "result").unwrap();

        assert!(e.project.abstractions[0].outputs.contains_key("result"));
        // Top-level wire from the instance's output follows.
        assert!(e
            .project
            .wire_nets
            .iter()
            .any(|n| n.source == Some(port("buf_1", "result"))));
        // The wrapper's dot target follows too.
        assert_eq!(
            e.project.abstractions[j].outputs.get("w"),
            Some(&"buf_1.result".to_string())
        );
        let mut p = e.project.clone();
        assert!(normalize(&mut p).is_empty(), "rename left inconsistencies");
    }

    #[test]
    fn rename_pin_rejects_collisions() {
        let mut e = abstraction_with_instance();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        assert!(matches!(
            e.rename_interface_pin(PortKind::Input, "x", "q").unwrap_err(),
            EditError::NameTaken(_)
        ));
        assert!(matches!(
            e.rename_interface_pin(PortKind::Input, "x", "has space")
                .unwrap_err(),
            EditError::InvalidName(_)
        ));
    }

    #[test]
    fn rename_abstraction_retypes_instances() {
        let mut e = abstraction_with_instance();
        e.rename_abstraction(0, "buffer").unwrap();
        assert_eq!(e.project.abstractions[0].name, "buffer");
        assert_eq!(e.project.components[1].component_type, "buffer");
        assert!(matches!(
            e.rename_abstraction(0, "nand").unwrap_err(),
            EditError::NameTaken(_)
        ));
        use crate::convert::project_to_circuit;
        assert!(project_to_circuit(&e.project).is_ok());
    }

    #[test]
    fn rename_project_allows_free_text() {
        let mut e = Editor::empty();
        e.rename_project("My First CPU!").unwrap();
        assert_eq!(e.project.name, "My First CPU!");
        assert!(matches!(
            e.rename_project("   ").unwrap_err(),
            EditError::InvalidName(_)
        ));
    }

    #[test]
    fn delete_abstraction_refused_while_used() {
        let mut e = abstraction_with_instance();
        assert!(matches!(
            e.delete_abstraction(0).unwrap_err(),
            EditError::AbstractionInUse(1)
        ));
        e.delete_components(&["buf_1".into()]).unwrap();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        e.delete_abstraction(0).unwrap();
        assert!(e.project.abstractions.is_empty());
        assert_eq!(e.sheet, SheetRef::Top, "deleting the open sheet goes home");
        // Undo brings it back (and the sheet restore comes from the stack).
        e.undo().unwrap();
        assert_eq!(e.project.abstractions.len(), 1);
    }

    #[test]
    fn pins_have_stored_positions_and_order_ports() {
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "a").unwrap();
        e.add_interface_pin(PortKind::Input, "b").unwrap();
        // Both pins got concrete homes at creation.
        assert!(e.project.abstractions[i].pin_positions.contains_key("a"));
        assert!(e.project.abstractions[i].pin_positions.contains_key("b"));

        // Move "b" above "a": the instance's ports reorder to match.
        let a_y = e.project.abstractions[i].pin_positions["a"][1];
        e.set_pin_position("b", [-4.0, a_y - 3.0]).unwrap();
        e.open_sheet(SheetRef::Top).unwrap();
        e.place_component("thing", [0, 0]).unwrap();
        let comp = &e.project.components[0];
        let ports = layout::component_ports(&e.project.abstractions, comp);
        let inputs: Vec<&str> = ports
            .iter()
            .filter(|p| p.info.kind == PortKind::Input)
            .map(|p| p.info.name.as_str())
            .collect();
        assert_eq!(inputs, vec!["b", "a"], "port order mirrors inside layout");

        // Rename keeps the position; removal drops it.
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.rename_interface_pin(PortKind::Input, "b", "top").unwrap();
        assert!(e.project.abstractions[i].pin_positions.contains_key("top"));
        e.remove_interface_pin(PortKind::Input, "top").unwrap();
        assert!(!e.project.abstractions[i].pin_positions.contains_key("top"));
    }

    // ── Clipboard ────────────────────────────────────────────────────────

    #[test]
    fn fragment_paste_preserves_internal_wiring() {
        let mut e = editor_with_two_nands();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 1.0]])
            .unwrap();
        e.place_note([0.0, 5.0], "keep me").unwrap();

        let frag = e.fragment(
            &["nand_1".into(), "nand_2".into()],
            &["note_1".into()],
        );
        let (new_comps, new_notes) = e.paste_fragment(&frag, [0, 8], true).unwrap();
        assert_eq!(new_comps, vec!["nand_3", "nand_4"]);
        assert_eq!(new_notes, vec!["note_2"]);

        // The copies carry their own wire, junction included, and the
        // original wiring is untouched.
        assert_eq!(e.project.wire_nets.len(), 2);
        let copied_net = e
            .project
            .wire_nets
            .iter()
            .find(|n| n.source == Some(port("nand_3", "out")))
            .expect("copied net rewired to fresh ids");
        assert_eq!(copied_net.sinks, vec![port("nand_4", "a")]);
        assert_eq!(copied_net.junctions.len(), 1);
        assert_eq!(copied_net.junctions[0].pos, [5.0, 9.0]);

        // One undo removes the whole paste.
        e.undo().unwrap();
        assert_eq!(e.project.components.len(), 2);
        assert_eq!(e.project.wire_nets.len(), 1);
        assert_eq!(e.project.notes.len(), 1);
    }

    #[test]
    fn fragment_excludes_wiring_to_uncopied_components() {
        let mut e = editor_with_two_nands();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
            .unwrap();
        // Copy only one endpoint: the wire between them must not come.
        let frag = e.fragment(&["nand_1".into()], &[]);
        assert!(frag.wire_nets.is_empty());
        let (ids, _) = e.paste_fragment(&frag, [0, 8], true).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(e.project.wire_nets.len(), 1, "no extra nets appeared");
    }

    #[test]
    fn paste_searches_for_free_space_and_rejects_recursion() {
        let mut e = editor_with_two_nands();
        let frag = e.fragment(&["nand_1".into()], &[]);
        // Zero offset collides with the original; search finds a spot.
        let (ids, _) = e.paste_fragment(&frag, [0, 0], true).unwrap();
        assert_eq!(ids.len(), 1);
        // Without search, a collision is refused outright.
        assert_eq!(
            e.paste_fragment(&frag, [0, 0], false).unwrap_err(),
            EditError::Overlap
        );

        // Recursion guard: an instance of X can't be pasted inside X.
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.place_component("thing", [0, 0]).unwrap();
        let frag = e.fragment(&["thing_1".into()], &[]);
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        assert!(matches!(
            e.paste_fragment(&frag, [0, 0], true).unwrap_err(),
            EditError::RecursiveAbstraction(_)
        ));
    }

    #[test]
    fn first_port_placement_freezes_the_rest_and_new_pins_find_slots() {
        use crate::project::PortSide;
        let mut e = Editor::empty();
        let i = e.create_abstraction("box").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        for name in ["a", "b", "c"] {
            e.add_interface_pin(PortKind::Input, name).unwrap();
        }
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        assert!(e.project.abstractions[i].port_layout.is_empty(), "auto until touched");

        // Positions before the first manual placement...
        let synth = ProjectComponent {
            id: String::new(),
            component_type: "box".into(),
            init: None,
            pos: [0.0, 0.0],
        };
        let before: Vec<_> = layout::component_ports(&e.project.abstractions, &synth)
            .into_iter()
            .filter(|p| p.info.name != "a")
            .map(|p| (p.info.name.clone(), p.pos))
            .collect();

        // ...moving "a" to the top edge must not move anyone else.
        e.set_port_layout(i, "a", PortSide::Top, 0.5).unwrap();
        let after: Vec<_> = layout::component_ports(&e.project.abstractions, &synth)
            .into_iter()
            .filter(|p| p.info.name != "a")
            .map(|p| (p.info.name.clone(), p.pos))
            .collect();
        assert_eq!(before, after, "freezing must not shuffle other ports");
        assert_eq!(e.project.abstractions[i].port_layout.len(), 4);

        // A new pin lands in a free slot instead of re-flowing the layout.
        e.add_interface_pin(PortKind::Input, "d").unwrap();
        let spots = &e.project.abstractions[i].port_layout;
        assert!(spots.contains_key("d"));
        let occupied: Vec<i64> = spots
            .values()
            .filter(|s| s.side == PortSide::Left)
            .map(|s| (s.t * e.project.abstractions[i].height - 0.5).round() as i64)
            .collect();
        let mut dedup = occupied.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(dedup.len(), occupied.len(), "no two left ports share a slot");
    }

    // ── Review regressions ───────────────────────────────────────────────

    #[test]
    fn expander_handles_right_to_left_wiring_inside_abstractions() {
        // H1: a sub-instance may reference one defined after it.
        let mut e = Editor::empty();
        let n = e.create_abstraction("inv").unwrap();
        e.open_sheet(SheetRef::Abstraction(n)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "b"), &[]).unwrap();
        e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[]).unwrap();

        let b = e.create_abstraction("buf2").unwrap();
        e.open_sheet(SheetRef::Abstraction(b)).unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        // Downstream instance placed FIRST, upstream second, wired
        // right-to-left: expansion must not depend on placement order.
        e.place_component("inv", [8, 0]).unwrap();
        e.place_component("inv", [0, 0]).unwrap();
        e.connect(port("inv_2", "q"), port("inv_1", "x"), &[]).unwrap();
        e.connect(port("inv_1", "q"), pin("OUTPUT:", "q"), &[]).unwrap();

        e.open_sheet(SheetRef::Top).unwrap();
        e.place_component("buf2", [0, 0]).unwrap();

        use crate::compiler::compile;
        use crate::convert::project_to_circuit;
        use crate::expander::expand;
        use crate::simulator::simulate;
        let expanded = expand(&project_to_circuit(&e.project).unwrap())
            .expect("wiring order must not matter");
        let compiled = compile(&expanded.def).unwrap();
        let results = simulate(&compiled);
        // inv_2 input unconnected → 0; NOT(0)=1; NOT(1)=0.
        let flat = expanded.aliases.get("buf2_1.q").unwrap();
        assert!(!results[0][flat]);
    }

    #[test]
    fn removing_a_pin_scrubs_instance_wiring_everywhere() {
        // H2: without the scrub, every later op on the top sheet fails.
        let mut e = abstraction_with_instance();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        e.remove_interface_pin(PortKind::Input, "x").unwrap();
        e.open_sheet(SheetRef::Top).unwrap();
        // No net may still reference the vanished port...
        for net in &e.project.wire_nets {
            assert!(!net.sinks.contains(&port("buf_1", "x")));
        }
        // ...and wiring on the top sheet still works.
        e.place_component("nand", [0, 10]).unwrap();
        let new_nand = e.project.components.last().unwrap().id.clone();
        e.connect(port("dff_1", "out"), port(&new_nand, "a"), &[])
            .expect("sheet must not be poisoned by the removed pin");
        // And the design still expands.
        use crate::convert::project_to_circuit;
        use crate::expander::expand;
        assert!(expand(&project_to_circuit(&e.project).unwrap()).is_ok());

        // Output pins scrub too, including dot targets elsewhere.
        let mut e = abstraction_with_instance();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        e.remove_interface_pin(PortKind::Output, "q").unwrap();
        for net in &e.project.wire_nets {
            assert_ne!(net.source, Some(port("buf_1", "q")));
        }
    }

    #[test]
    fn cyclic_abstractions_in_json_do_not_crash() {
        // H3: a → b → a cycle from a hand-edited file.
        let mut e = Editor::empty();
        e.create_abstraction("a").unwrap();
        e.create_abstraction("b").unwrap();
        e.create_abstraction("c").unwrap();
        e.project.abstractions[0].components.push(ProjectComponent {
            id: "x".into(),
            component_type: "b".into(),
            init: None,
            pos: [0.0, 0.0],
        });
        e.project.abstractions[1].components.push(ProjectComponent {
            id: "y".into(),
            component_type: "a".into(),
            init: None,
            pos: [0.0, 0.0],
        });
        let mut p = e.project.clone();
        normalize(&mut p); // must terminate
        e.open_sheet(SheetRef::Abstraction(2)).unwrap();
        // Placing into the cycle-adjacent sheet must not blow the stack.
        e.place_component("a", [0, 0]).unwrap();
        assert!(matches!(
            e.place_component("c", [10, 0]).unwrap_err(),
            EditError::RecursiveAbstraction(_)
        ));
    }

    #[test]
    fn copies_never_stay_connected_to_unselected_things() {
        // Wires to interface pins are outside the selection, so they're
        // cut at copy time — same-sheet paste included.
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Input, "x").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
        let frag = e.fragment(&["nand_1".into()], &[]);
        assert!(
            frag.wire_nets.is_empty(),
            "the pin is not selected, so its wire must not travel"
        );

        // Same-sheet paste: the copy is NOT wired to the pin.
        let (ids, _) = e.paste_fragment(&frag, [0, 4], true).unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(e.project.abstractions[i].wire_nets.len(), 1);
        let net = &e.project.abstractions[i].wire_nets[0];
        assert_eq!(net.sinks, vec![port("nand_1", "a")], "original only");

        // Cross-sheet paste works too.
        e.open_sheet(SheetRef::Top).unwrap();
        let (ids, _) = e.paste_fragment(&frag, [0, 0], true).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(e.project.wire_nets.is_empty());
    }

    #[test]
    fn normalize_canonicalizes_output_targets() {
        // M3: hand-written "gate.out" on a primitive, and bare instance.
        let mut e = Editor::empty();
        let i = e.create_abstraction("thing").unwrap();
        e.open_sheet(SheetRef::Abstraction(i)).unwrap();
        e.add_interface_pin(PortKind::Output, "q").unwrap();
        e.place_component("nand", [4, 0]).unwrap();
        let mut p = e.project.clone();
        p.abstractions[i].outputs.insert("q".into(), "nand_1.out".into());
        let notes = normalize(&mut p);
        assert_eq!(p.abstractions[i].outputs["q"], "nand_1");
        assert!(!notes.is_empty());
    }

    // ── Normalization ────────────────────────────────────────────────────

    #[test]
    fn normalize_synthesizes_segments_for_bare_nets() {
        let mut e = editor_with_two_nands();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
            .unwrap();
        let mut project = e.project.clone();
        // Strip the visual half, as a hand-written file might.
        project.wire_nets[0].segments.clear();
        let notes = normalize(&mut project);
        assert_eq!(project.wire_nets[0].segments.len(), 1);
        assert!(notes.is_empty(), "synthesis is not a repair note: {notes:?}");
    }

    #[test]
    fn normalize_merges_same_source_nets_and_dedups_drivers() {
        let mut project = editor_with_two_nands().project;
        let seg_less = |id: &str, sink: PortRef| WireNet {
            id: id.into(),
            source: Some(port("nand_1", "out")),
            sinks: vec![sink],
            junctions: vec![],
            segments: vec![],
        };
        project.wire_nets = vec![
            seg_less("w1", port("nand_2", "a")),
            seg_less("w2", port("nand_2", "b")),
            seg_less("w3", port("nand_2", "a")), // duplicate driver for a
        ];
        let notes = normalize(&mut project);
        assert_eq!(project.wire_nets.len(), 1);
        assert_eq!(project.wire_nets[0].sinks.len(), 2);
        assert!(!notes.is_empty());
    }

    #[test]
    fn normalize_repairs_dangling_references() {
        let mut project = editor_with_two_nands().project;
        project.wire_nets = vec![WireNet {
            id: "w1".into(),
            source: Some(port("ghost", "out")),
            sinks: vec![port("nand_2", "a"), port("nand_2", "bogus_port")],
            junctions: vec![],
            segments: vec![],
        }];
        let notes = normalize(&mut project);
        // Source and bogus sink dropped; sourceless net keeps its one valid
        // sink but connects nothing → collected.
        assert!(project.wire_nets.is_empty());
        assert!(notes.len() >= 2);
    }

    #[test]
    fn normalize_accepts_example_files() {
        for path in ["examples/toggle.json", "examples/counter.json"] {
            let text = std::fs::read_to_string(path).unwrap();
            let mut project: Project = serde_json::from_str(&text).unwrap();
            normalize(&mut project);
            // Every sink must have a visual path after normalization.
            for net in &project.wire_nets {
                for sink in &net.sinks {
                    assert!(
                        net.segments.iter().any(|seg| [&seg.from, &seg.to]
                            .iter()
                            .any(|e| matches!(e, Endpoint::Port { component_id, port_name }
                                if component_id == &sink.component_id
                                    && port_name == &sink.port_name))),
                        "{path}: sink {sink:?} has no segment"
                    );
                }
            }
        }
    }
}
