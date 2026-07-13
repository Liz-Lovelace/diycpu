//! Sheet plumbing: uniform access to the top circuit or an abstraction
//! body, port direction/kind resolution, and id generation (ids are
//! always derived from the document, never from a counter).


use std::collections::HashMap;

use crate::layout::{self, PortKind};
use crate::project::{PortRef, Project, ProjectComponent, WireNet};

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
