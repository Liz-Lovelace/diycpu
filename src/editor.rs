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
    SecondClock,
    ClockInsideAbstraction,
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
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponentType(t) => write!(f, "Unknown component type \"{t}\""),
            Self::UnknownComponent(id) => write!(f, "No component \"{id}\""),
            Self::UnknownPort(c, p) => write!(f, "Component \"{c}\" has no port \"{p}\""),
            Self::Overlap => write!(f, "Components can't overlap"),
            Self::SecondClock => write!(f, "The design already has a clock (one per design)"),
            Self::ClockInsideAbstraction => {
                write!(f, "Clocks can only be placed in the main circuit")
            }
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
        })
    }

    // ── Sheet access ─────────────────────────────────────────────────────

    /// The components and wire nets of the current sheet.
    pub fn sheet_data(&self) -> (&[ProjectComponent], &[WireNet]) {
        sheet_of(&self.project, self.sheet)
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

            if type_name == "clk" {
                match sheet {
                    SheetRef::Top => {
                        if project.components.iter().any(|c| c.component_type == "clk") {
                            return Err(EditError::SecondClock);
                        }
                    }
                    SheetRef::Abstraction(_) => {
                        return Err(EditError::ClockInsideAbstraction)
                    }
                }
            }

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
                init: (type_name == "dff").then_some(false),
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
        if delta == [0, 0] {
            return Ok(()); // not an edit; don't spend an undo step
        }
        let ids = ids.to_vec();
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
            Ok(())
        })
    }

    /// Delete components and scrub every reference to them from wire nets.
    pub fn delete_components(&mut self, ids: &[String]) -> Result<(), EditError> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids = ids.to_vec();
        self.mutate(move |project, sheet| {
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            for id in &ids {
                if !components.iter().any(|c| &c.id == id) {
                    return Err(EditError::UnknownComponent(id.clone()));
                }
            }
            components.retain(|c| !ids.contains(&c.id));

            for net in wire_nets.iter_mut() {
                if net
                    .source
                    .as_ref()
                    .is_some_and(|s| ids.contains(&s.component_id))
                {
                    net.source = None;
                }
                net.sinks.retain(|s| !ids.contains(&s.component_id));
                net.segments.retain(|seg| {
                    ![&seg.from, &seg.to].iter().any(|e| match e {
                        Endpoint::Port { component_id, .. } => ids.contains(component_id),
                        Endpoint::Junction { .. } => false,
                    })
                });
                prune_net_geometry(net);
            }
            gc_nets(wire_nets);

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
            Ok(())
        })
    }

    /// Connect two ports with a wire, optionally bending through `bends`
    /// (grid points, in order from `a` to `b`). One port must be an output
    /// and the other an input, in either argument order. If the input is
    /// already driven, the old connection is replaced. Fan-out merges into
    /// the output's existing net.
    ///
    /// Inside an abstraction sheet, `INPUT:<name>` pseudo-ports (input pins)
    /// act as outputs, and `OUTPUT:<name>` pseudo-ports (output pins) act as
    /// inputs — connecting to an output pin sets the abstraction's `outputs`
    /// map entry rather than creating a net.
    pub fn connect(
        &mut self,
        a: PortRef,
        b: PortRef,
        bends: &[[f32; 2]],
    ) -> Result<(), EditError> {
        let bends = bends.to_vec();
        self.mutate(move |project, sheet| {
            let (components, _) = sheet_of(project, sheet).ok_or(EditError::BadSheet)?;
            let interface = match sheet {
                SheetRef::Abstraction(i) => project.abstractions.get(i),
                SheetRef::Top => None,
            };

            let kind_of = |port: &PortRef| -> Result<PortKind, EditError> {
                if let Some(name) = port.component_id.strip_prefix(crate::convert::INPUT_PREFIX)
                {
                    let def = interface.ok_or(EditError::NotAnAbstractionSheet)?;
                    return if def.inputs.iter().any(|i| i == name) {
                        Ok(PortKind::Output) // input pins drive wires
                    } else {
                        Err(EditError::UnknownComponent(port.component_id.clone()))
                    };
                }
                if let Some(name) = port.component_id.strip_prefix(layout::OUTPUT_PREFIX) {
                    let def = interface.ok_or(EditError::NotAnAbstractionSheet)?;
                    return if def.outputs.contains_key(name) {
                        Ok(PortKind::Input) // output pins are fed by wires
                    } else {
                        Err(EditError::UnknownComponent(port.component_id.clone()))
                    };
                }
                let comp = components
                    .iter()
                    .find(|c| c.id == port.component_id)
                    .ok_or_else(|| EditError::UnknownComponent(port.component_id.clone()))?;
                layout::type_ports(&project.abstractions, &comp.component_type)
                    .and_then(|ports| {
                        ports.into_iter().find(|p| p.name == port.port_name)
                    })
                    .map(|p| p.kind)
                    .ok_or_else(|| {
                        EditError::UnknownPort(
                            port.component_id.clone(),
                            port.port_name.clone(),
                        )
                    })
            };

            let (output, input, bends) = match (kind_of(&a)?, kind_of(&b)?) {
                (PortKind::Output, PortKind::Input) => (a, b, bends),
                (PortKind::Input, PortKind::Output) => {
                    let mut rev = bends.clone();
                    rev.reverse();
                    (b, a, rev)
                }
                _ => return Err(EditError::NotAnInputAndOutput),
            };

            // Wiring into an output pin: record the connection in the
            // abstraction's outputs map. Its wire is drawn derived from that
            // map, so there is no net (and no stored bends) to maintain.
            if let Some(pin_name) = input.component_id.strip_prefix(layout::OUTPUT_PREFIX) {
                if output.component_id.starts_with(crate::convert::INPUT_PREFIX) {
                    return Err(EditError::PassthroughNotAllowed);
                }
                let source_comp = components
                    .iter()
                    .find(|c| c.id == output.component_id)
                    .ok_or_else(|| EditError::UnknownComponent(output.component_id.clone()))?;
                let target = if crate::types::is_primitive(&source_comp.component_type) {
                    source_comp.id.clone()
                } else {
                    format!("{}.{}", output.component_id, output.port_name)
                };
                let pin_name = pin_name.to_string();
                let SheetRef::Abstraction(i) = sheet else {
                    return Err(EditError::NotAnAbstractionSheet);
                };
                project.abstractions[i].outputs.insert(pin_name, target);
                return Ok(());
            }

            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;

            // An input has exactly one driver: replace any existing one.
            remove_sink(wire_nets, &input);

            // One net per source: merge fan-out into the existing net.
            let net_index = match wire_nets
                .iter()
                .position(|n| n.source.as_ref() == Some(&output))
            {
                Some(i) => i,
                None => {
                    let id = generate_net_id(wire_nets);
                    wire_nets.push(WireNet {
                        id,
                        source: Some(output.clone()),
                        sinks: vec![],
                        junctions: vec![],
                        segments: vec![],
                    });
                    wire_nets.len() - 1
                }
            };
            let net = &mut wire_nets[net_index];

            // Build the visual path: output → bends… → input.
            let mut previous = Endpoint::Port {
                component_id: output.component_id.clone(),
                port_name: output.port_name.clone(),
            };
            for bend in &bends {
                let junction_id = generate_junction_id(net);
                net.junctions.push(Junction {
                    id: junction_id.clone(),
                    pos: *bend,
                });
                let junction = Endpoint::Junction { junction_id };
                net.segments.push(crate::project::Segment {
                    from: previous,
                    to: junction.clone(),
                });
                previous = junction;
            }
            net.segments.push(crate::project::Segment {
                from: previous,
                to: Endpoint::Port {
                    component_id: input.component_id.clone(),
                    port_name: input.port_name.clone(),
                },
            });

            net.sinks.push(input);
            Ok(())
        })
    }

    /// Remove one input's connection (the rest of the net stays).
    pub fn disconnect_sink(&mut self, sink: PortRef) -> Result<(), EditError> {
        self.mutate(move |project, sheet| {
            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            remove_sink(wire_nets, &sink);
            gc_nets(wire_nets);
            Ok(())
        })
    }

    /// Delete a whole wire net.
    pub fn delete_net(&mut self, net_id: &str) -> Result<(), EditError> {
        let net_id = net_id.to_string();
        self.mutate(move |project, sheet| {
            let (_, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            wire_nets.retain(|n| n.id != net_id);
            Ok(())
        })
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
            if comp.component_type != "dff" {
                return Err(EditError::NotADff(id.clone()));
            }
            let new = !comp.init.unwrap_or(false);
            comp.init = Some(new);
            Ok(new)
        })
    }

    /// Duplicate components, placing the copies at the first free spot
    /// offset down-right from the originals. Returns the new IDs.
    pub fn duplicate_components(&mut self, ids: &[String]) -> Result<Vec<String>, EditError> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let ids = ids.to_vec();
        self.mutate(move |project, sheet| {
            let abstractions = project.abstractions.clone();
            let (components, _) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;

            let originals: Vec<ProjectComponent> = ids
                .iter()
                .map(|id| {
                    components
                        .iter()
                        .find(|c| &c.id == id)
                        .cloned()
                        .ok_or_else(|| EditError::UnknownComponent(id.clone()))
                })
                .collect::<Result<_, _>>()?;

            // Find an offset where every copy fits.
            let mut offset = None;
            'search: for step in 1..100 {
                let d = step as f32;
                let candidate_rects: Vec<Rect> = originals
                    .iter()
                    .map(|c| {
                        let mut r = layout::footprint(&abstractions, c);
                        r.x += d;
                        r.y += d;
                        r
                    })
                    .collect();
                let clear = components.iter().all(|existing| {
                    let er = layout::footprint(&abstractions, existing);
                    !candidate_rects.iter().any(|r| r.intersects(&er))
                });
                if clear {
                    offset = Some(d);
                    break 'search;
                }
            }
            let d = offset.ok_or(EditError::Overlap)?;

            let mut new_ids = Vec::new();
            for original in originals {
                let id = generate_id(components, &original.component_type);
                components.push(ProjectComponent {
                    id: id.clone(),
                    component_type: original.component_type.clone(),
                    init: original.init,
                    pos: [original.pos[0] + d, original.pos[1] + d],
                });
                new_ids.push(id);
            }
            Ok(new_ids)
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
                || project.abstractions.iter().any(|a| a.name == name)
            {
                return Err(EditError::NameTaken(name.clone()));
            }
            let [width, height] = layout::default_abstraction_size(&name, 0, 0);
            project.abstractions.push(crate::project::ProjectAbstraction {
                name: name.clone(),
                inputs: vec![],
                outputs: HashMap::new(),
                width,
                height,
                components: vec![],
                wire_nets: vec![],
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
            match kind {
                PortKind::Input => def.inputs.push(name.clone()),
                PortKind::Output => {
                    def.outputs.insert(name.clone(), String::new());
                }
            }
            update_interface_height(def);
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
                        if net
                            .source
                            .as_ref()
                            .is_some_and(|s| s.component_id == pseudo_id)
                        {
                            net.source = None;
                        }
                        net.segments.retain(|seg| {
                            ![&seg.from, &seg.to].iter().any(|e| {
                                matches!(e, Endpoint::Port { component_id, .. }
                                    if *component_id == pseudo_id)
                            })
                        });
                        prune_net_geometry(net);
                    }
                    gc_nets(&mut def.wire_nets);
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
            update_interface_height(def);
            Ok(())
        })
    }
}

/// Keep an abstraction's instance footprint tall enough for its pins.
fn update_interface_height(def: &mut crate::project::ProjectAbstraction) {
    let needed = def.inputs.len().max(def.outputs.len()).max(2) as f32;
    if def.height < needed {
        def.height = needed;
    }
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

/// Does abstraction `name` (transitively) instantiate `target`?
/// Also true when `name == target`.
pub fn abstraction_uses(
    abstractions: &[crate::project::ProjectAbstraction],
    name: &str,
    target: &str,
) -> bool {
    if name == target {
        return true;
    }
    let Some(def) = abstractions.iter().find(|a| a.name == name) else {
        return false; // primitives and unknown types contain nothing
    };
    def.components
        .iter()
        .any(|c| abstraction_uses(abstractions, &c.component_type, target))
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

fn generate_junction_id(net: &WireNet) -> String {
    for n in 1.. {
        let candidate = format!("j_{n}");
        if !net.junctions.iter().any(|j| j.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

// ── Net surgery helpers ──────────────────────────────────────────────────────

/// Remove `sink` from whichever net drives it, along with the visual path
/// that led to it. Nets left without sinks are handled by `gc_nets`.
fn remove_sink(wire_nets: &mut [WireNet], sink: &PortRef) {
    for net in wire_nets.iter_mut() {
        if !net.sinks.contains(sink) {
            continue;
        }
        net.sinks.retain(|s| s != sink);
        net.segments.retain(|seg| {
            ![&seg.from, &seg.to].iter().any(|e| {
                matches!(e, Endpoint::Port { component_id, port_name }
                    if *component_id == sink.component_id && *port_name == sink.port_name)
            })
        });
        prune_net_geometry(net);
    }
}

/// Remove dead visual geometry from a net: junctions that dangle (fewer than
/// two incident segments) are dissolved together with their segments, until
/// the net's junction graph only contains real through-points and branches.
fn prune_net_geometry(net: &mut WireNet) {
    loop {
        let degree: HashMap<&str, usize> = {
            let mut d: HashMap<&str, usize> = HashMap::new();
            for seg in &net.segments {
                for e in [&seg.from, &seg.to] {
                    if let Endpoint::Junction { junction_id } = e {
                        *d.entry(junction_id.as_str()).or_default() += 1;
                    }
                }
            }
            d
        };
        let dangling: Vec<String> = net
            .junctions
            .iter()
            .filter(|j| degree.get(j.id.as_str()).copied().unwrap_or(0) < 2)
            .map(|j| j.id.clone())
            .collect();
        if dangling.is_empty() {
            return;
        }
        net.junctions.retain(|j| !dangling.contains(&j.id));
        net.segments.retain(|seg| {
            ![&seg.from, &seg.to].iter().any(|e| {
                matches!(e, Endpoint::Junction { junction_id } if dangling.contains(junction_id))
            })
        });
    }
}

/// Drop nets that no longer connect anything: a net only exists while it
/// joins a source to at least one sink.
fn gc_nets(wire_nets: &mut Vec<WireNet>) {
    wire_nets.retain(|n| n.source.is_some() && !n.sinks.is_empty());
}

// ── Normalization (repair pass for loaded / hand-edited files) ───────────────

/// Bring a project into the editor's invariants, repairing what can be
/// repaired. Returns human-readable notes about anything that was fixed.
/// Hand-written files without visual routing get port-to-port segments
/// synthesized so they render like any interactively drawn wire.
pub fn normalize(project: &mut Project) -> Vec<String> {
    let mut notes = Vec::new();
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

    // Output pin targets must reference an existing output port.
    for a in &mut project.abstractions {
        let comps = a.components.clone();
        for (pin, target) in a.outputs.iter_mut() {
            if target.is_empty() {
                continue; // unwired pin — legal while editing
            }
            let port = layout::output_target_port(target);
            let ok = comps
                .iter()
                .find(|c| c.id == port.component_id)
                .and_then(|c| layout::type_ports(&abstractions, &c.component_type))
                .map_or(false, |ports| {
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
            }
        }
    }
    notes
}

fn normalize_sheet(
    abstractions: &[crate::project::ProjectAbstraction],
    components: &mut Vec<ProjectComponent>,
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

    // Drop logical references that don't resolve or have the wrong direction.
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

    // Merge nets that share a source (one net per source).
    let mut i = 0;
    while i < wire_nets.len() {
        if wire_nets[i].source.is_none() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < wire_nets.len() {
            if wire_nets[j].source == wire_nets[i].source {
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

    // One driver per input: first net wins.
    let mut seen_sinks: Vec<PortRef> = Vec::new();
    for net in wire_nets.iter_mut() {
        let net_id = net.id.clone();
        net.sinks.retain(|s| {
            if seen_sinks.contains(s) {
                notes.push(format!(
                    "{label}: input {}.{} had two drivers; kept the first (dropped from \"{net_id}\")",
                    s.component_id, s.port_name
                ));
                false
            } else {
                seen_sinks.push(s.clone());
                true
            }
        });
    }

    for net in wire_nets.iter_mut() {
        // Drop segments whose endpoints don't resolve.
        let before = net.segments.len();
        let ids: Vec<String> = components.iter().map(|c| c.id.clone()).collect();
        let junction_ids: Vec<String> = net.junctions.iter().map(|j| j.id.clone()).collect();
        net.segments.retain(|seg| {
            [&seg.from, &seg.to].iter().all(|e| match e {
                Endpoint::Port { component_id, .. } => {
                    ids.contains(component_id)
                        || component_id.starts_with(crate::convert::INPUT_PREFIX)
                }
                Endpoint::Junction { junction_id } => junction_ids.contains(junction_id),
            })
        });
        if net.segments.len() != before {
            notes.push(format!(
                "{label}: wire \"{}\" had dangling segments; removed them",
                net.id
            ));
        }
        prune_net_geometry(net);

        // Synthesize a direct segment for any sink with no visual path.
        if let Some(source) = net.source.clone() {
            let sinks = net.sinks.clone();
            for sink in sinks {
                let has_path = net.segments.iter().any(|seg| {
                    [&seg.from, &seg.to].iter().any(|e| {
                        matches!(e, Endpoint::Port { component_id, port_name }
                            if *component_id == sink.component_id
                                && *port_name == sink.port_name)
                    })
                });
                if !has_path {
                    net.segments.push(crate::project::Segment {
                        from: Endpoint::Port {
                            component_id: source.component_id.clone(),
                            port_name: source.port_name.clone(),
                        },
                        to: Endpoint::Port {
                            component_id: sink.component_id.clone(),
                            port_name: sink.port_name.clone(),
                        },
                    });
                }
            }
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
fn merge_net(net: &mut WireNet, mut absorbed: WireNet) {
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
            junction.id = new_id;
        }
        net.junctions.push(junction.clone());
    }
    net.sinks.append(&mut absorbed.sinks);
    net.segments.append(&mut absorbed.segments);
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
    fn only_one_clock() {
        let mut e = Editor::empty();
        e.place_component("clk", [0, 0]).unwrap();
        assert_eq!(
            e.place_component("clk", [10, 0]).unwrap_err(),
            EditError::SecondClock
        );
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
    fn delete_component_scrubs_nets_and_junctions() {
        let mut e = editor_with_two_nands();
        e.place_component("clk", [0, 10]).unwrap();
        e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 1.0]])
            .unwrap();
        e.connect(port("nand_1", "out"), port("nand_2", "b"), &[])
            .unwrap();
        e.delete_components(&["nand_2".into()]).unwrap();
        // Net lost both sinks → gone entirely, junction included.
        assert!(e.project.wire_nets.is_empty());
        assert_eq!(e.project.components.len(), 2);
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
        // The junction on the removed branch is pruned, the other stays.
        assert_eq!(net.junctions.len(), 1);
        assert_eq!(net.junctions[0].pos, [5.0, 3.0]);
        // Disconnecting the last sink removes the net.
        e.disconnect_sink(port("nand_2", "b")).unwrap();
        assert!(e.project.wire_nets.is_empty());
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
        assert_eq!(e.toggle_dff_init("dff_1").unwrap(), true);
        assert_eq!(e.toggle_dff_init("dff_1").unwrap(), false);
        e.place_component("nand", [10, 0]).unwrap();
        assert!(matches!(
            e.toggle_dff_init("nand_1").unwrap_err(),
            EditError::NotADff(_)
        ));
    }

    #[test]
    fn duplicate_places_clear_of_everything() {
        let mut e = editor_with_two_nands();
        let new_ids = e
            .duplicate_components(&["nand_1".into(), "nand_2".into()])
            .unwrap();
        assert_eq!(new_ids, vec!["nand_3", "nand_4"]);
        assert_eq!(e.project.components.len(), 4);
        // No overlaps anywhere.
        let abs = &e.project.abstractions;
        for (i, a) in e.project.components.iter().enumerate() {
            for b in &e.project.components[i + 1..] {
                assert!(!layout::footprint(abs, a).intersects(&layout::footprint(abs, b)));
            }
        }
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
        });
        e
    }

    #[test]
    fn no_clock_inside_abstraction() {
        let mut e = project_with_not_abstraction();
        e.open_sheet(SheetRef::Abstraction(0)).unwrap();
        assert_eq!(
            e.place_component("clk", [8, 0]).unwrap_err(),
            EditError::ClockInsideAbstraction
        );
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
        assert_eq!(session_result[0]["buf2_1.out"], false);
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

        let mut project = e.project.clone();
        // A hand-edited file referencing a pin that doesn't exist.
        project.abstractions[i].wire_nets[0].source = Some(pin("INPUT:", "ghost"));
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
