//! Sheet plumbing: uniform access to the top circuit or an abstraction
//! body, port direction/kind resolution, and id generation (ids are
//! always derived from the document, never from a counter).


use std::collections::HashMap;

use crate::layout::{self, PortKind};
use crate::project::{Endpoint, PortRef, Project, ProjectComponent, WireNet};

use super::*;


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

pub(crate) fn sheet_mut(
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

pub(crate) fn sheet_notes_mut(
    project: &mut Project,
    sheet: SheetRef,
) -> Option<&mut Vec<crate::project::Note>> {
    match sheet {
        SheetRef::Top => Some(&mut project.notes),
        SheetRef::Abstraction(i) => project.abstractions.get_mut(i).map(|a| &mut a.notes),
    }
}

pub fn sheet_images(
    project: &Project,
    sheet: SheetRef,
) -> Option<&[crate::project::MediaImage]> {
    match sheet {
        SheetRef::Top => Some(project.images.as_slice()),
        SheetRef::Abstraction(i) => project.abstractions.get(i).map(|a| a.images.as_slice()),
    }
}

pub(crate) fn sheet_images_mut(
    project: &mut Project,
    sheet: SheetRef,
) -> Option<&mut Vec<crate::project::MediaImage>> {
    match sheet {
        SheetRef::Top => Some(&mut project.images),
        SheetRef::Abstraction(i) => project.abstractions.get_mut(i).map(|a| &mut a.images),
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

pub(crate) fn generate_id(components: &[ProjectComponent], type_name: &str) -> String {
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

pub(crate) fn generate_net_id(wire_nets: &[WireNet]) -> String {
    for n in 1.. {
        let candidate = format!("net_{n}");
        if !wire_nets.iter().any(|w| w.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

pub(crate) fn generate_note_id(notes: &[crate::project::Note]) -> String {
    for n in 1.. {
        let candidate = format!("note_{n}");
        if !notes.iter().any(|note| note.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

pub(crate) fn generate_image_id(images: &[crate::project::MediaImage]) -> String {
    for n in 1.. {
        let candidate = format!("img_{n}");
        if !images.iter().any(|img| img.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

pub(crate) fn generate_junction_id(net: &WireNet) -> String {
    for n in 1.. {
        let candidate = format!("j_{n}");
        if !net.junctions.iter().any(|j| j.id == candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Port-kind resolution context, captured up front so operations can hold
/// mutable borrows of the sheet while classifying ports.
/// Answers "what is this port?" for one sheet — direction (in/out) and
/// wire kind (signal/bundle) — with the interface pseudo-ports
/// (`INPUT:x` / `OUTPUT:x`) included. Ops capture one of these before
/// mutating so validation always sees a consistent snapshot.
pub(crate) struct PortResolver {
    pub(crate) abstractions: Vec<crate::project::ProjectAbstraction>,
    pub(crate) interface_inputs: Option<Vec<String>>,
    pub(crate) interface_outputs: Option<Vec<String>>,
    pub(crate) interface_bundles: HashMap<String, u32>,
}

impl PortResolver {
    pub(crate) fn new(project: &Project, sheet: SheetRef) -> Result<Self, EditError> {
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
        let interface_bundles = match sheet {
            SheetRef::Top => HashMap::new(),
            SheetRef::Abstraction(i) => project
                .abstractions
                .get(i)
                .map(|d| d.bundle_pins.clone())
                .unwrap_or_default(),
        };
        Ok(PortResolver {
            abstractions: project.abstractions.clone(),
            interface_inputs,
            interface_outputs,
            interface_bundles,
        })
    }

    /// Signal or bundle — for any port, interface pseudo-ports included.
    pub(crate) fn wire_kind_of(
        &self,
        components: &[ProjectComponent],
        port: &PortRef,
    ) -> Result<layout::WireKind, EditError> {
        let pin_name = port
            .component_id
            .strip_prefix(crate::convert::INPUT_PREFIX)
            .or_else(|| port.component_id.strip_prefix(layout::OUTPUT_PREFIX));
        if let Some(name) = pin_name {
            return Ok(match self.interface_bundles.get(name) {
                Some(w) => layout::WireKind::Bundle(*w),
                None => layout::WireKind::Signal,
            });
        }
        let comp = components
            .iter()
            .find(|c| c.id == port.component_id)
            .ok_or_else(|| EditError::UnknownComponent(port.component_id.clone()))?;
        layout::component_port_info(&self.abstractions, comp)
            .and_then(|ports| ports.into_iter().find(|p| p.name == port.port_name))
            .map(|p| p.wire)
            .ok_or_else(|| {
                EditError::UnknownPort(port.component_id.clone(), port.port_name.clone())
            })
    }

    /// Wire-direction kind of a port, including interface pseudo-ports.
    pub(crate) fn kind_of(
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
        layout::component_port_info(&self.abstractions, comp)
            .and_then(|ports| ports.into_iter().find(|p| p.name == port.port_name))
            .map(|p| p.kind)
            .ok_or_else(|| {
                EditError::UnknownPort(port.component_id.clone(), port.port_name.clone())
            })
    }
}


// ── The settle pass ──────────────────────────────────────────────────────────

/// Bring every derived field of the whole project in line with the wire
/// graph: repair each sheet's nets (orphans, island splits, source/sink
/// derivation, empty-net collection) and re-derive each abstraction's
/// outputs map from the nets that sink into its output pins. Runs after
/// EVERY successful operation, from `Editor::mutate`.
pub(crate) fn settle_project(project: &mut Project) -> Result<(), EditError> {
    for index in 0..=project.abstractions.len() {
        let sheet = if index == 0 {
            SheetRef::Top
        } else {
            SheetRef::Abstraction(index - 1)
        };
        let ctx = PortResolver::new(project, sheet)?;
        let (components, wire_nets) = sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
        let components: Vec<ProjectComponent> = components.clone();
        repair_nets(wire_nets, &components, &ctx)?;
    }
    derive_outputs(project);
    Ok(())
}

/// The engine-facing form of "this port drives something": bare id for
/// single-output components, dotted for instance outputs, `INPUT:x`
/// verbatim for passthroughs.
pub(crate) fn output_target_form(
    components: &[ProjectComponent],
    source: &PortRef,
) -> String {
    if source.component_id.starts_with(crate::convert::INPUT_PREFIX) {
        return source.component_id.clone();
    }
    match components.iter().find(|c| c.id == source.component_id) {
        Some(c) if crate::types::is_primitive(&c.component_type)
            || c.component_type == "const" =>
        {
            source.component_id.clone()
        }
        _ => format!("{}.{}", source.component_id, source.port_name),
    }
}

/// Each abstraction's `outputs` map is DERIVED: pin names are the keys
/// (declarations — they persist unwired), values come from whichever net
/// sinks into the pin. Bundle pins keep "" — the lowering pass reads
/// their nets directly and writes per-lane entries.
pub(crate) fn derive_outputs(project: &mut Project) {
    for a in project.abstractions.iter_mut() {
        let nets = &a.wire_nets;
        let components = &a.components;
        let mut derived: Vec<(String, String)> = Vec::new();
        for name in a.outputs.keys() {
            if a.bundle_pins.contains_key(name) {
                derived.push((name.clone(), String::new()));
                continue;
            }
            let pin_id = format!("{}{name}", crate::convert::OUTPUT_PREFIX);
            let target = nets
                .iter()
                .find(|n| n.sinks.iter().any(|s| s.component_id == pin_id))
                .and_then(|n| n.source.as_ref())
                .map(|src| output_target_form(components, src))
                .unwrap_or_default();
            derived.push((name.clone(), target));
        }
        for (name, target) in derived {
            a.outputs.insert(name, target);
        }
    }
}

// ── Invariants (debug + stress tests) ────────────────────────────────────────

/// Everything that must hold of a settled document. Checked after every
/// operation in debug builds, and after every step of the stress tests.
pub(crate) fn check_invariants(project: &Project) -> Result<(), String> {
    for index in 0..=project.abstractions.len() {
        let sheet = if index == 0 {
            SheetRef::Top
        } else {
            SheetRef::Abstraction(index - 1)
        };
        let Some((components, wire_nets)) = sheet_of(project, sheet) else {
            continue;
        };
        let label = match sheet {
            SheetRef::Top => "top".to_string(),
            SheetRef::Abstraction(i) => project.abstractions[i].name.clone(),
        };
        let ctx = PortResolver::new(project, sheet).map_err(|e| e.to_string())?;

        let mut net_ids = std::collections::HashSet::new();
        for net in wire_nets {
            if !net_ids.insert(&net.id) {
                return Err(format!("{label}: duplicate net id {}", net.id));
            }
            if net.segments.is_empty() {
                return Err(format!("{label}/{}: empty net survived gc", net.id));
            }
            let mut junction_ids = std::collections::HashSet::new();
            for j in &net.junctions {
                if !junction_ids.insert(&j.id) {
                    return Err(format!("{label}/{}: duplicate junction {}", net.id, j.id));
                }
            }
            // Every endpoint resolves; junctions referenced exist.
            for seg in &net.segments {
                for e in [&seg.from, &seg.to] {
                    match e {
                        Endpoint::Junction { junction_id } => {
                            if !net.junctions.iter().any(|j| &j.id == junction_id) {
                                return Err(format!(
                                    "{label}/{}: segment references missing junction {junction_id}",
                                    net.id
                                ));
                            }
                        }
                        Endpoint::Port {
                            component_id,
                            port_name,
                        } => {
                            let port = PortRef {
                                component_id: component_id.clone(),
                                port_name: port_name.clone(),
                            };
                            if ctx.kind_of(components, &port).is_err() {
                                return Err(format!(
                                    "{label}/{}: segment references missing port {component_id}.{port_name}",
                                    net.id
                                ));
                            }
                        }
                    }
                }
            }
            // No orphan junctions.
            for j in &net.junctions {
                let touched = net.segments.iter().any(|seg| {
                    [&seg.from, &seg.to].iter().any(
                        |e| matches!(e, Endpoint::Junction { junction_id } if *junction_id == j.id),
                    )
                });
                if !touched {
                    return Err(format!("{label}/{}: orphan junction {}", net.id, j.id));
                }
            }
            // One connected island per net.
            let mut probe = net.clone();
            if !split_off_islands(&mut probe).is_empty() {
                return Err(format!("{label}/{}: net is several islands", net.id));
            }
            // Derived logic matches a fresh derivation.
            let mut fresh = net.clone();
            sync_net_logic(&mut fresh, components, &ctx).map_err(|e| {
                format!("{label}/{}: does not re-sync: {e}", net.id)
            })?;
            if fresh.source != net.source || fresh.sinks != net.sinks {
                return Err(format!(
                    "{label}/{}: stale derived logic (stored {:?}→{:?}, fresh {:?}→{:?})",
                    net.id, net.source, net.sinks, fresh.source, fresh.sinks
                ));
            }
        }
    }
    // The outputs maps match a fresh derivation.
    let mut fresh = project.clone();
    derive_outputs(&mut fresh);
    for (a, b) in project.abstractions.iter().zip(fresh.abstractions.iter()) {
        if a.outputs != b.outputs {
            return Err(format!(
                "abstraction \"{}\": stale outputs map ({:?} vs derived {:?})",
                a.name, a.outputs, b.outputs
            ));
        }
    }
    Ok(())
}
