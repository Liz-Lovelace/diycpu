//! Abstraction definitions and their interfaces: creating, renaming and
//! deleting abstractions; interface pins and their layout on the closed
//! rectangle.


use std::collections::HashMap;

use crate::layout::{self, PortKind};
use crate::project::{Endpoint, PortRef};

use super::*;


impl Editor {
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
                bundle_pins: HashMap::new(),
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
            images: vec![],
            });
            Ok(project.abstractions.len() - 1)
        })
    }

    /// Add an input or output pin to the abstraction being edited.
    /// An output pin starts unwired (empty target) until connected.
    /// Add an interface pin; `bundle` makes it carry that many lines.
    pub fn add_interface_pin(
        &mut self,
        kind: PortKind,
        name: &str,
        bundle: Option<u32>,
    ) -> Result<(), EditError> {
        let name = name.to_string();
        self.mutate(move |project, sheet| {
            let SheetRef::Abstraction(i) = sheet else {
                return Err(EditError::NotAnAbstractionSheet);
            };
            if let Some(w) = bundle {
                if !(2..=32).contains(&w) {
                    return Err(EditError::BadBundleWidth);
                }
            }
            let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
            if !valid_name(&name) {
                return Err(EditError::InvalidName(name.clone()));
            }
            if def.inputs.iter().any(|x| x == &name) || def.outputs.contains_key(&name) {
                return Err(EditError::NameTaken(name.clone()));
            }
            if let Some(w) = bundle {
                def.bundle_pins.insert(name.clone(), w);
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
                    // Output pins that forwarded this input become unwired.
                    for target in def.outputs.values_mut() {
                        if *target == pseudo_id {
                            target.clear();
                        }
                    }
                    for net in def.wire_nets.iter_mut() {
                        net.segments.retain(|seg| {
                            ![&seg.from, &seg.to].iter().any(|e| {
                                matches!(e, Endpoint::Port { component_id, .. }
                                    if *component_id == pseudo_id)
                            })
                        });
                    }
                    let ctx = PortResolver {
                        abstractions: abstractions_snapshot,
                        interface_inputs: Some(def.inputs.clone()),
                        interface_outputs: Some(def.outputs.keys().cloned().collect()),
                        interface_bundles: def.bundle_pins.clone(),
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
            def.bundle_pins.remove(&name);
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
                let ctx = PortResolver {
                    abstractions: abstractions_snapshot.clone(),
                    interface_inputs: interface.map(|a| a.inputs.clone()),
                    interface_outputs: interface
                        .map(|a| a.outputs.keys().cloned().collect()),
                    interface_bundles: interface
                        .map(|a| a.bundle_pins.clone())
                        .unwrap_or_default(),
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
            if let Some(w) = def.bundle_pins.remove(&old) {
                def.bundle_pins.insert(new.clone(), w);
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
                    // Output pins forwarding this input follow the rename.
                    for target in def.outputs.values_mut() {
                        if *target == old_id {
                            *target = new_id.clone();
                        }
                    }
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
pub(crate) fn ensure_interface_fits(def: &mut crate::project::ProjectAbstraction) {
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
