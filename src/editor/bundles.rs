//! Lane-count settings for bundlers, unbundlers and bundle pins.



use crate::layout::{self};
use crate::project::Endpoint;

use super::*;


impl Editor {
    /// Change a bundler/unbundler's lane count. Wires on removed lanes and
    /// on the (re-sized) bundle port are disconnected.
    pub fn set_bundle_width(&mut self, id: &str, width: u32) -> Result<(), EditError> {
        let id = id.to_string();
        self.mutate(move |project, sheet| {
            if !(2..=32).contains(&width) {
                return Err(EditError::BadBundleWidth);
            }
            let ctx = KindCtx::new(project, sheet)?;
            let abstractions = project.abstractions.clone();
            let (components, wire_nets) =
                sheet_mut(project, sheet).ok_or(EditError::BadSheet)?;
            let index = components
                .iter()
                .position(|c| c.id == id)
                .ok_or_else(|| EditError::UnknownComponent(id.clone()))?;
            if !layout::is_bundle_component(&components[index].component_type) {
                return Err(EditError::NotABundle(id.clone()));
            }
            let old = layout::bits_of(&components[index]);
            if old == width {
                return Ok(());
            }
            components[index].bits = Some(width);

            // Growing the body must not run into neighbours.
            let rect = layout::footprint(&abstractions, &components[index]);
            for other in components.iter().filter(|c| c.id != id) {
                if layout::footprint(&abstractions, other).intersects(&rect) {
                    return Err(EditError::Overlap);
                }
            }

            // Wires on lanes that no longer exist go away; the bundle port
            // changed width, so its bundle wire goes too (reconnect it).
            for net in wire_nets.iter_mut() {
                net.segments.retain(|seg| {
                    ![&seg.from, &seg.to].iter().any(|e| {
                        matches!(e, Endpoint::Port { component_id, port_name }
                            if *component_id == id
                                && (*port_name == "bundle"
                                    || port_name.parse::<u32>()
                                        .is_ok_and(|k| k >= width)))
                    })
                });
            }
            repair_nets(wire_nets, components, &ctx)?;
            Ok(())
        })
    }

    /// Make an interface pin carry a bundle (or a single signal again).
    /// Everything wired to the pin — inside the sheet and on every
    /// instance — is disconnected: its kind changed.
    pub fn set_pin_bundle_width(
        &mut self,
        name: &str,
        width: Option<u32>,
    ) -> Result<(), EditError> {
        let name = name.to_string();
        self.mutate(move |project, sheet| {
            if let Some(w) = width {
                if !(2..=32).contains(&w) {
                    return Err(EditError::BadBundleWidth);
                }
            }
            let SheetRef::Abstraction(i) = sheet else {
                return Err(EditError::NotAnAbstractionSheet);
            };
            let def = project.abstractions.get_mut(i).ok_or(EditError::BadSheet)?;
            let is_input = def.inputs.iter().any(|x| x == &name);
            if !is_input && !def.outputs.contains_key(&name) {
                return Err(EditError::UnknownComponent(name.clone()));
            }
            let before = def.bundle_pins.get(&name).copied();
            if before == width {
                return Ok(());
            }
            match width {
                Some(w) => {
                    def.bundle_pins.insert(name.clone(), w);
                }
                None => {
                    def.bundle_pins.remove(&name);
                }
            }
            // A passthrough output forwarding this pin can't survive a
            // kind change.
            let pseudo_in = format!("{}{name}", crate::convert::INPUT_PREFIX);
            for target in def.outputs.values_mut() {
                if *target == pseudo_in {
                    target.clear();
                }
            }
            if !is_input {
                if let Some(t) = def.outputs.get_mut(&name) {
                    t.clear();
                }
            }
            let abs_name = def.name.clone();

            // Scrub everything attached to the pin: its pseudo-port inside
            // this sheet, and the matching port on every instance.
            let pseudo_out = format!("{}{name}", layout::OUTPUT_PREFIX);
            let abstractions_snapshot = project.abstractions.clone();
            let sheet_count = project.abstractions.len() + 1;
            for sheet_index in 0..sheet_count {
                let (sheet_components, sheet_nets) = if sheet_index == 0 {
                    (&project.components, &mut project.wire_nets)
                } else {
                    let a = &mut project.abstractions[sheet_index - 1];
                    (&a.components, &mut a.wire_nets)
                };
                let own_sheet =
                    sheet_index > 0 && abstractions_snapshot[sheet_index - 1].name == abs_name;
                let instances: Vec<String> = sheet_components
                    .iter()
                    .filter(|c| c.component_type == abs_name)
                    .map(|c| c.id.clone())
                    .collect();
                if instances.is_empty() && !own_sheet {
                    continue;
                }
                for net in sheet_nets.iter_mut() {
                    net.segments.retain(|seg| {
                        ![&seg.from, &seg.to].iter().any(|e| {
                            matches!(e, Endpoint::Port { component_id, port_name }
                                if (instances.contains(component_id) && *port_name == name)
                                    || (own_sheet
                                        && (*component_id == pseudo_in
                                            || *component_id == pseudo_out)))
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
            Ok(())
        })
    }
}


