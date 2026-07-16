//! The repair pass for loaded and hand-edited files: bring any project
//! into the invariants the editor maintains, fixing what can be fixed
//! and reporting what was changed.


use std::collections::HashMap;

use crate::layout::{self, PortKind};
use crate::project::{Endpoint, PortRef, Project, ProjectComponent, WireNet};

use super::*;


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
        let (label, interface_inputs, interface_outputs, components, wire_nets): (
            String,
            Option<Vec<String>>,
            Option<Vec<String>>,
            _,
            _,
        ) = if sheet_index == 0 {
            (
                "main circuit".to_string(),
                None,
                None,
                &mut project.components,
                &mut project.wire_nets,
            )
        } else {
            let a = &mut project.abstractions[sheet_index - 1];
            (
                format!("abstraction \"{}\"", a.name),
                Some(a.inputs.clone()),
                Some(a.outputs.keys().cloned().collect()),
                &mut a.components,
                &mut a.wire_nets,
            )
        };
        let interface_bundles = if sheet_index == 0 {
            HashMap::new()
        } else {
            abstractions[sheet_index - 1].bundle_pins.clone()
        };
        normalize_sheet(
            &abstractions,
            components,
            wire_nets,
            interface_inputs,
            interface_outputs,
            interface_bundles,
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

    // The outputs map is DERIVED from the nets that sink into output
    // pins. Files from before that change (or hand-written ones) carry
    // targets with no corresponding net: synthesize one per target — the
    // per-sheet pass above already gave every net segments, so these run
    // through a second, quieter settle below.
    for a in &mut project.abstractions {
        for (pin, target) in a.outputs.clone() {
            if target.is_empty() || a.bundle_pins.contains_key(&pin) {
                continue;
            }
            let pin_id = format!("{}{pin}", crate::convert::OUTPUT_PREFIX);
            let already_wired = a.wire_nets.iter().any(|n| {
                n.segments.iter().any(|seg| {
                    [&seg.from, &seg.to].iter().any(|e| {
                        matches!(e, Endpoint::Port { component_id, .. }
                            if *component_id == pin_id)
                    })
                })
            });
            if already_wired {
                continue;
            }
            let source = if let Some(name) = target.strip_prefix(crate::convert::INPUT_PREFIX)
            {
                PortRef {
                    component_id: target.clone(),
                    port_name: name.to_string(),
                }
            } else {
                layout::output_target_port(&target)
            };
            let id = generate_net_id(&a.wire_nets);
            a.wire_nets.push(WireNet {
                bundle: false,
                id,
                source: Some(source),
                sinks: vec![PortRef {
                    component_id: pin_id,
                    port_name: pin.clone(),
                }],
                junctions: vec![],
                segments: vec![],
            });
        }
    }
    // Settle the synthesized nets (and report targets that don't resolve).
    for i in 0..project.abstractions.len() {
        let a = &mut project.abstractions[i];
        let label = format!("abstraction \"{}\"", a.name);
        let inputs = Some(a.inputs.clone());
        let outputs = Some(a.outputs.keys().cloned().collect());
        let bundles = a.bundle_pins.clone();
        let (components, wire_nets) = (&mut a.components, &mut a.wire_nets);
        normalize_sheet(
            &abstractions,
            components,
            wire_nets,
            inputs,
            outputs,
            bundles,
            &label,
            &mut notes,
        );
    }
    derive_outputs(project);

    // Early display components were separate "seg"/"face" types; they are
    // one "segment" type with a variant now.
    {
        let sheets = std::iter::once(&mut project.components)
            .chain(project.abstractions.iter_mut().map(|a| &mut a.components));
        for components in sheets {
            for c in components.iter_mut() {
                match c.component_type.as_str() {
                    "seg" => {
                        c.component_type = "segment".into();
                        notes.push(format!("\"{}\" updated to the segment type", c.id));
                    }
                    "face" => {
                        c.component_type = "segment".into();
                        c.bits = Some(10);
                        notes.push(format!(
                            "\"{}\" updated to the segment type (face variant)",
                            c.id
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    // Bundle pins must name a real pin and carry a sane lane count.
    for a in &mut project.abstractions {
        let inputs = a.inputs.clone();
        let output_names: Vec<String> = a.outputs.keys().cloned().collect();
        a.bundle_pins.retain(|name, w| {
            let exists =
                inputs.iter().any(|i| i == name) || output_names.iter().any(|o| o == name);
            if !exists {
                notes.push(format!(
                    "abstraction \"{}\": bundle entry for missing pin \"{name}\" dropped",
                    a.name
                ));
                return false;
            }
            if !(2..=32).contains(w) {
                notes.push(format!(
                    "abstraction \"{}\": bundle pin \"{name}\" width clamped",
                    a.name
                ));
                *w = (*w).clamp(2, 32);
            }
            true
        });
    }

    // Images: unique non-empty ids, finite positive sizes.
    sanitize_images(&mut project.images, "main circuit", &mut notes);
    for i in 0..project.abstractions.len() {
        let label = format!("abstraction \"{}\"", project.abstractions[i].name);
        sanitize_images(&mut project.abstractions[i].images, &label, &mut notes);
    }
    notes
}

pub(crate) fn sanitize_images(
    images: &mut [crate::project::MediaImage],
    label: &str,
    notes: &mut Vec<String>,
) {
    for k in 0..images.len() {
        let dup = images[..k].iter().any(|o| o.id == images[k].id);
        if images[k].id.is_empty() || dup {
            let fresh = generate_image_id(images);
            notes.push(format!("{label}: image given a fresh id \"{fresh}\""));
            images[k].id = fresh;
        }
        let s = images[k].size;
        let fixed = [
            if s[0].is_finite() { s[0].clamp(0.5, 60.0) } else { 6.0 },
            if s[1].is_finite() { s[1].clamp(0.5, 60.0) } else { 4.0 },
        ];
        if fixed != s {
            notes.push(format!(
                "{label}: image \"{}\" size repaired",
                images[k].id
            ));
            images[k].size = fixed;
        }
        for v in images[k].pos.iter_mut() {
            if !v.is_finite() {
                *v = 0.0;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn normalize_sheet(
    abstractions: &[crate::project::ProjectAbstraction],
    components: &mut [ProjectComponent],
    wire_nets: &mut Vec<WireNet>,
    interface_inputs: Option<Vec<String>>,
    interface_outputs: Option<Vec<String>>,
    interface_bundles: HashMap<String, u32>,
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
        // OUTPUT:q pseudo-ports (output pins) are sinks of their sheet.
        if let Some(name) = port.component_id.strip_prefix(crate::convert::OUTPUT_PREFIX) {
            return match &interface_outputs {
                Some(outputs) if outputs.iter().any(|o| o == name) => Some(PortKind::Input),
                _ => None,
            };
        }
        let comp = components.iter().find(|c| c.id == port.component_id)?;
        layout::component_port_info(abstractions, comp)?
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
    let ctx = PortResolver {
        abstractions: abstractions.to_vec(),
        interface_outputs,
        interface_inputs,
        interface_bundles,
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
