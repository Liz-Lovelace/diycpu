use std::collections::HashMap;
use std::fmt;

use crate::project::{PortRef, Project, ProjectAbstraction, ProjectComponent, WireNet};
use crate::types::{is_primitive, AbstractionDef, CircuitDef, ComponentDef};

#[derive(Debug)]
pub enum ConvertError {
    /// A bundle resolves through itself (bundler → unbundler → back).
    BundleLoop { component_id: String },
    /// A sink port references a component that doesn't exist.
    UnknownSinkComponent {
        wire_net_id: String,
        component_id: String,
    },
    /// A source port references a component that doesn't exist.
    UnknownSourceComponent {
        wire_net_id: String,
        component_id: String,
    },
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BundleLoop { component_id } => write!(
                f,
                "Bundle wiring around \"{component_id}\" feeds back into itself"
            ),
            Self::UnknownSinkComponent {
                wire_net_id,
                component_id,
            } => write!(
                f,
                "Wire net \"{wire_net_id}\" sink references unknown component \"{component_id}\""
            ),
            Self::UnknownSourceComponent {
                wire_net_id,
                component_id,
            } => write!(
                f,
                "Wire net \"{wire_net_id}\" source references unknown component \"{component_id}\""
            ),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Prefix for pseudo-component IDs representing abstraction input pins in wire nets.
pub const INPUT_PREFIX: &str = "INPUT:";

/// Convert a Project to an engine CircuitDef.
///
/// This extracts logical connections from wire nets (source→sink pairs)
/// and maps them into engine ComponentDef input HashMaps.
pub fn project_to_circuit(project: &Project) -> Result<CircuitDef, ConvertError> {
    // Bundles are a purely visual grouping: lower them to their individual
    // lanes first, so the engine only ever sees plain wires.
    let lowered;
    let project = if project_needs_lowering(project) {
        lowered = lower_bundles(project)?;
        &lowered
    } else {
        project
    };
    let abstractions = project
        .abstractions
        .iter()
        .map(convert_abstraction)
        .collect::<Result<Vec<_>, _>>()?;

    let components =
        build_engine_components(&project.components, &project.wire_nets)?;

    Ok(CircuitDef {
        name: project.name.clone(),
        steps: project.steps,
        abstractions,
        components,
    })
}


// ── Bundle lowering ──────────────────────────────────────────────────────────
//
// Bundlers, unbundlers, bundle wires and bundle interface pins are GUI
// concepts. This pass rewrites a project into an equivalent one with only
// plain signals: every bundle lane k is resolved structurally to the port
// that drives it, bundle pins become per-lane pins (`x__b0` …), and the
// bundle layer is deleted. The engine pipeline never knows it existed.

fn is_bundle_comp(c: &ProjectComponent) -> bool {
    matches!(c.component_type.as_str(), "bundle" | "unbundle")
}

fn comp_bits(c: &ProjectComponent) -> u32 {
    c.bits.unwrap_or(8).clamp(2, 32)
}

fn lane_name(pin: &str, k: u32) -> String {
    format!("{pin}__b{k}")
}

fn is_display_comp(c: &ProjectComponent) -> bool {
    matches!(c.component_type.as_str(), "light" | "rgb" | "seg" | "face")
}

fn project_needs_lowering(p: &Project) -> bool {
    let sheet_has = |comps: &[ProjectComponent], nets: &[WireNet]| {
        comps.iter().any(|c| is_bundle_comp(c) || is_display_comp(c))
            || nets.iter().any(|n| n.bundle)
    };
    sheet_has(&p.components, &p.wire_nets)
        || p.abstractions
            .iter()
            .any(|a| sheet_has(&a.components, &a.wire_nets) || !a.bundle_pins.is_empty())
}

/// Displays are pure sinks that exist to be looked at: delete them and
/// every wire reference to them before the bundle lowering runs, so the
/// rest of the pipeline never has to know they exist.
fn strip_displays(components: &mut Vec<ProjectComponent>, wire_nets: &mut [WireNet]) {
    let ids: Vec<String> = components
        .iter()
        .filter(|c| is_display_comp(c))
        .map(|c| c.id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    components.retain(|c| !is_display_comp(c));
    for net in wire_nets.iter_mut() {
        net.segments.retain(|seg| {
            ![&seg.from, &seg.to].iter().any(|e| {
                matches!(e, crate::project::Endpoint::Port { component_id, .. }
                    if ids.contains(component_id))
            })
        });
        net.sinks.retain(|s| !ids.contains(&s.component_id));
    }
}

fn lower_bundles(original: &Project) -> Result<Project, ConvertError> {
    let mut p = original.clone();

    strip_displays(&mut p.components, &mut p.wire_nets);
    for a in &mut p.abstractions {
        strip_displays(&mut a.components, &mut a.wire_nets);
    }

    // Bundle interface pins become per-lane pins. (`bundle_pins` itself is
    // kept as reference data; the engine conversion ignores it.)
    for a in &mut p.abstractions {
        let mut new_inputs = Vec::new();
        for name in &a.inputs {
            match a.bundle_pins.get(name) {
                Some(w) => new_inputs.extend((0..*w).map(|k| lane_name(name, k))),
                None => new_inputs.push(name.clone()),
            }
        }
        a.inputs = new_inputs;
        for (name, w) in &a.bundle_pins {
            if a.outputs.remove(name).is_some() {
                for k in 0..*w {
                    a.outputs.insert(lane_name(name, k), String::new());
                }
            }
        }
    }

    let defs = original.abstractions.clone();
    lower_sheet(
        &defs,
        &std::collections::HashMap::new(),
        &mut p.components,
        &mut p.wire_nets,
        None,
    )?;
    for a in &mut p.abstractions {
        let own = a.bundle_pins.clone();
        lower_sheet(
            &defs,
            &own,
            &mut a.components,
            &mut a.wire_nets,
            Some(&mut a.outputs),
        )?;
    }

    // A bundle output pin always presents ALL its lanes: any lane its
    // sheet doesn't drive (a narrower bundle wired in, or nothing at all)
    // reads as constant 0 — same rule as unconnected inputs everywhere.
    for a in &mut p.abstractions {
        for (pin, w) in a.bundle_pins.clone() {
            if !a.outputs.contains_key(&lane_name(&pin, 0)) {
                continue; // an input pin
            }
            for k in 0..w {
                let lane = lane_name(&pin, k);
                if a.outputs.get(&lane).is_none_or(|t| t.is_empty()) {
                    let zero = format!("__zero_{lane}");
                    a.components.push(ProjectComponent {
                        display: None,
                        id: zero.clone(),
                        component_type: "dff".into(),
                        init: Some(false),
                        bits: None,
                        pos: [0.0, 0.0],
                    });
                    a.outputs.insert(lane, zero);
                }
            }
        }
    }
    Ok(p)
}

fn lower_sheet(
    defs: &[ProjectAbstraction],
    own_bundles: &std::collections::HashMap<String, u32>,
    components: &mut Vec<ProjectComponent>,
    wire_nets: &mut Vec<WireNet>,
    mut outputs: Option<&mut std::collections::HashMap<String, String>>,
) -> Result<(), ConvertError> {
    use crate::project::Junction;
    let comps: Vec<ProjectComponent> = components.clone();
    let comp_of = |id: &str| comps.iter().find(|c| c.id == id);
    let nets: Vec<WireNet> = wire_nets.clone();

    let width_of = |port: &PortRef| -> Option<u32> {
        if let Some(name) = port
            .component_id
            .strip_prefix(INPUT_PREFIX)
            .or_else(|| port.component_id.strip_prefix("OUTPUT:"))
        {
            return own_bundles.get(name).copied();
        }
        let c = comp_of(&port.component_id)?;
        if is_bundle_comp(c) {
            return Some(comp_bits(c));
        }
        defs.iter()
            .find(|a| a.name == c.component_type)?
            .bundle_pins
            .get(&port.port_name)
            .copied()
    };

    // The port that actually drives lane k of a bundle source — following
    // chains of unbundle → bundle if needed.
    fn resolve_lane(
        src: &PortRef,
        k: u32,
        comps: &[ProjectComponent],
        nets: &[WireNet],
        depth: usize,
    ) -> Result<Option<PortRef>, ConvertError> {
        if depth > 64 {
            return Err(ConvertError::BundleLoop {
                component_id: src.component_id.clone(),
            });
        }
        if let Some(name) = src.component_id.strip_prefix(INPUT_PREFIX) {
            let lane = lane_name(name, k);
            return Ok(Some(PortRef {
                component_id: format!("{INPUT_PREFIX}{lane}"),
                port_name: lane,
            }));
        }
        let Some(comp) = comps.iter().find(|c| c.id == src.component_id) else {
            return Ok(None);
        };
        if comp.component_type == "bundle" {
            // Lane k = whatever signal net feeds the bundler's input k.
            let lane_port = k.to_string();
            let feed = nets
                .iter()
                .filter(|n| !n.bundle)
                .find(|n| {
                    n.sinks
                        .iter()
                        .any(|s| s.component_id == comp.id && s.port_name == lane_port)
                })
                .and_then(|n| n.source.clone());
            let Some(feed) = feed else { return Ok(None) };
            // A feed from an unbundler output continues the chain.
            resolve_signal(&feed, comps, nets, depth + 1)
        } else {
            // An abstraction instance's bundle output: its per-lane port.
            Ok(Some(PortRef {
                component_id: src.component_id.clone(),
                port_name: lane_name(&src.port_name, k),
            }))
        }
    }

    // A signal-level source that may sit on an unbundler output: follow it
    // back through the unbundler's bundle feed.
    fn resolve_signal(
        port: &PortRef,
        comps: &[ProjectComponent],
        nets: &[WireNet],
        depth: usize,
    ) -> Result<Option<PortRef>, ConvertError> {
        if depth > 64 {
            return Err(ConvertError::BundleLoop {
                component_id: port.component_id.clone(),
            });
        }
        let Some(comp) = comps.iter().find(|c| c.id == port.component_id) else {
            return Ok(Some(port.clone())); // pseudo-ports and pins pass through
        };
        if comp.component_type != "unbundle" {
            return Ok(Some(port.clone()));
        }
        let Ok(k) = port.port_name.parse::<u32>() else {
            return Ok(None);
        };
        let feed = nets
            .iter()
            .filter(|n| n.bundle)
            .find(|n| {
                n.sinks
                    .iter()
                    .any(|s| s.component_id == comp.id && s.port_name == "bundle")
            })
            .and_then(|n| n.source.clone());
        let Some(feed) = feed else { return Ok(None) };
        resolve_lane(&feed, k, comps, nets, depth + 1)
    }

    // The engine-facing outputs-map target string for a lane source.
    let target_of = |port: &PortRef| -> String {
        if port.component_id.starts_with(INPUT_PREFIX) {
            return port.component_id.clone();
        }
        match comp_of(&port.component_id) {
            Some(c) if is_primitive(&c.component_type) || c.component_type == "const" => {
                port.component_id.clone()
            }
            _ => format!("{}.{}", port.component_id, port.port_name),
        }
    };

    // Walk every bundle net and materialize its lanes.
    let mut source_redirects: Vec<(PortRef, Option<PortRef>)> = Vec::new();
    let mut target_rewrites: Vec<(String, String)> = Vec::new();
    let mut synthetic: Vec<WireNet> = Vec::new();
    for bn in nets.iter().filter(|n| n.bundle) {
        let Some(src) = &bn.source else { continue };
        let Some(w) = width_of(src) else { continue };
        for sink in &bn.sinks {
            let w = width_of(sink).unwrap_or(w).min(w);
            if let Some(pin) = sink.component_id.strip_prefix("OUTPUT:") {
                // A bundle output pin: each lane pin forwards its source.
                for k in 0..w {
                    if let Some(s) = resolve_lane(src, k, &comps, &nets, 0)? {
                        target_rewrites.push((lane_name(pin, k), target_of(&s)));
                    }
                }
                continue;
            }
            let Some(comp) = comp_of(&sink.component_id) else { continue };
            if comp.component_type == "unbundle" {
                // Every net driven by this unbundler's lane outputs is
                // re-sourced to the lane's true driver.
                for k in 0..w {
                    let s = resolve_lane(src, k, &comps, &nets, 0)?;
                    source_redirects.push((
                        PortRef {
                            component_id: comp.id.clone(),
                            port_name: k.to_string(),
                        },
                        s,
                    ));
                }
            } else {
                // An instance's bundle input: bind each per-lane input.
                for k in 0..w {
                    if let Some(s) = resolve_lane(src, k, &comps, &nets, 0)? {
                        synthetic.push(WireNet {
                            bundle: false,
                            id: format!("__lane_{}_{}_{k}", bn.id, sink.component_id),
                            source: Some(s),
                            sinks: vec![PortRef {
                                component_id: sink.component_id.clone(),
                                port_name: lane_name(&sink.port_name, k),
                            }],
                            junctions: Vec::<Junction>::new(),
                            segments: vec![],
                        });
                    }
                }
            }
        }
    }

    // Apply: re-source unbundler-driven nets, rewrite output targets.
    for net in wire_nets.iter_mut() {
        if let Some(current) = &net.source {
            if let Some((_, new_src)) = source_redirects.iter().find(|(from, _)| from == current)
            {
                net.source = new_src.clone();
            }
        }
    }
    if let Some(outputs) = outputs.as_deref_mut() {
        // Output pins that dotted into an unbundler lane ("u1.3").
        for target in outputs.values_mut() {
            if let Some((id, port)) = target.split_once('.') {
                let from = PortRef {
                    component_id: id.to_string(),
                    port_name: port.to_string(),
                };
                if let Some((_, new_src)) =
                    source_redirects.iter().find(|(f, _)| *f == from)
                {
                    *target = match new_src {
                        Some(s) => target_of(s),
                        None => String::new(),
                    };
                }
            }
        }
        for (pin, target) in &target_rewrites {
            outputs.insert(pin.clone(), target.clone());
        }
    }
    wire_nets.extend(synthetic);

    // Delete the bundle layer and scrub what pointed into it.
    let bundle_ids: Vec<String> = comps
        .iter()
        .filter(|c| is_bundle_comp(c))
        .map(|c| c.id.clone())
        .collect();
    components.retain(|c| !is_bundle_comp(c));
    wire_nets.retain(|n| !n.bundle);
    for net in wire_nets.iter_mut() {
        if net
            .source
            .as_ref()
            .is_some_and(|s| bundle_ids.contains(&s.component_id))
        {
            net.source = None; // an unfed unbundler lane reads as 0
        }
        net.sinks.retain(|s| !bundle_ids.contains(&s.component_id));
    }
    if let Some(outputs) = outputs {
        for target in outputs.values_mut() {
            if !target.is_empty() {
                let id = target.split_once('.').map(|(i, _)| i).unwrap_or(target);
                if bundle_ids.iter().any(|b| b == id) {
                    target.clear();
                }
            }
        }
    }
    Ok(())
}

/// Build engine ComponentDefs from project components, resolving connections from wire nets.
fn build_engine_components(
    project_components: &[ProjectComponent],
    wire_nets: &[WireNet],
) -> Result<Vec<ComponentDef>, ConvertError> {
    // Build a set of known component IDs for validation.
    let known_ids: HashMap<&str, &ProjectComponent> = project_components
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    // Build connection map: (sink_component_id, sink_port_name) -> source_reference_string
    let mut connection_map: HashMap<(&str, &str), String> = HashMap::new();

    for net in wire_nets {
        let Some(source) = &net.source else {
            continue; // No source — unconnected net, skip.
        };

        let source_ref =
            build_source_reference(source, &known_ids, &net.id)?;

        for sink in &net.sinks {
            // Validate sink component exists (unless it's an INPUT: pseudo-component).
            if !sink.component_id.starts_with(INPUT_PREFIX)
                && !known_ids.contains_key(sink.component_id.as_str())
            {
                return Err(ConvertError::UnknownSinkComponent {
                    wire_net_id: net.id.clone(),
                    component_id: sink.component_id.clone(),
                });
            }

            connection_map.insert(
                (sink.component_id.as_str(), sink.port_name.as_str()),
                source_ref.clone(),
            );
        }
    }

    let mut engine_components = Vec::new();
    for pc in project_components {
        let mut inputs = HashMap::new();

        // Collect all connections targeting this component.
        for ((comp_id, port_name), source_ref) in &connection_map {
            if *comp_id == pc.id.as_str() {
                inputs.insert(port_name.to_string(), source_ref.clone());
            }
        }

        // A `const` is exactly a DFF that never latches: same init-driven
        // output, no inputs. Lower it here so the engine stays simple.
        let (engine_type, init) = if pc.component_type == "const" {
            ("dff".to_string(), pc.init.or(Some(false)))
        } else {
            (pc.component_type.clone(), pc.init)
        };
        engine_components.push(ComponentDef {
            component_type: engine_type,
            name: pc.id.clone(),
            init,
            inputs,
        });
    }

    Ok(engine_components)
}

/// Build the engine source reference string for a port ref.
///
/// - For single-output primitives (clk, nand, dff): just the component ID.
/// - For abstraction instances: "component_id.output_name" (dot notation).
/// - For INPUT: pseudo-components: strip the prefix and return the input name.
fn build_source_reference(
    source: &PortRef,
    known_components: &HashMap<&str, &ProjectComponent>,
    wire_net_id: &str,
) -> Result<String, ConvertError> {
    // Handle INPUT: pseudo-component (used inside abstraction sub-circuits).
    if let Some(input_name) = source.component_id.strip_prefix(INPUT_PREFIX) {
        return Ok(input_name.to_string());
    }

    let comp = known_components.get(source.component_id.as_str()).ok_or_else(|| {
        ConvertError::UnknownSourceComponent {
            wire_net_id: wire_net_id.to_string(),
            component_id: source.component_id.clone(),
        }
    })?;

    if is_primitive(&comp.component_type) {
        // Primitives have a single output; reference by component ID only.
        Ok(comp.id.clone())
    } else {
        // Abstraction instance: use dot notation for specific output.
        Ok(format!("{}.{}", comp.id, source.port_name))
    }
}

/// Convert a ProjectAbstraction to an engine AbstractionDef.
fn convert_abstraction(pa: &ProjectAbstraction) -> Result<AbstractionDef, ConvertError> {
    let components =
        build_engine_components(&pa.components, &pa.wire_nets)?;

    Ok(AbstractionDef {
        name: pa.name.clone(),
        inputs: pa.inputs.clone(),
        // Unwired pins are dropped (an editing state, not an error);
        // `INPUT:x` passthrough targets lower to the bare input name the
        // expander resolves through the instance's bindings.
        outputs: pa
            .outputs
            .iter()
            .filter(|(_, target)| !target.is_empty())
            .map(|(name, target)| {
                let lowered = target.strip_prefix(INPUT_PREFIX).unwrap_or(target);
                (name.clone(), lowered.to_string())
            })
            .collect(),
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::*;

    fn make_component(id: &str, comp_type: &str) -> ProjectComponent {
        ProjectComponent {
            display: None,
            bits: None,
            id: id.into(),
            component_type: comp_type.into(),
            init: None,
            pos: [0.0, 0.0],
        }
    }

    fn make_port_ref(component_id: &str, port_name: &str) -> PortRef {
        PortRef {
            component_id: component_id.into(),
            port_name: port_name.into(),
        }
    }

    fn simple_wire_net(id: &str, source: PortRef, sinks: Vec<PortRef>) -> WireNet {
        WireNet {
            bundle: false,
            id: id.into(),
            source: Some(source),
            sinks,
            junctions: vec![],
            segments: vec![],
        }
    }

    #[test]
    fn convert_simple_circuit() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 4,
            abstractions: vec![],
            components: vec![
                make_component("clk_1", "clk"),
                {
                    let mut c = make_component("dff_1", "dff");
                    c.init = Some(false);
                    c
                },
                make_component("nand_1", "nand"),
            ],
            wire_nets: vec![
                simple_wire_net(
                    "w1",
                    make_port_ref("nand_1", "out"),
                    vec![make_port_ref("dff_1", "d")],
                ),
                simple_wire_net(
                    "w2",
                    make_port_ref("clk_1", "out"),
                    vec![make_port_ref("dff_1", "clk")],
                ),
                simple_wire_net(
                    "w3",
                    make_port_ref("dff_1", "out"),
                    vec![
                        make_port_ref("nand_1", "a"),
                        make_port_ref("nand_1", "b"),
                    ],
                ),
            ],
        };

        let circuit = project_to_circuit(&project).unwrap();

        assert_eq!(circuit.name, "test");
        assert_eq!(circuit.steps, 4);
        assert_eq!(circuit.components.len(), 3);

        // clk_1 has no inputs.
        assert!(circuit.components[0].inputs.is_empty());

        // dff_1 has d=nand_1, clk=clk_1.
        assert_eq!(circuit.components[1].inputs["d"], "nand_1");
        assert_eq!(circuit.components[1].inputs["clk"], "clk_1");

        // nand_1 has a=dff_1, b=dff_1.
        assert_eq!(circuit.components[2].inputs["a"], "dff_1");
        assert_eq!(circuit.components[2].inputs["b"], "dff_1");
    }

    #[test]
    fn convert_abstraction_with_dot_notation() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 0,
            abstractions: vec![ProjectAbstraction {
                bundle_pins: HashMap::new(),
                pin_positions: HashMap::new(),
                port_layout: HashMap::new(),
                notes: vec![],
            images: vec![],
                name: "not".into(),
                inputs: vec!["x".into()],
                outputs: {
                    let mut m = HashMap::new();
                    m.insert("out".into(), "gate".into());
                    m
                },
                width: 140.0,
                height: 70.0,
                components: vec![make_component("gate", "nand")],
                wire_nets: vec![simple_wire_net(
                    "aw1",
                    make_port_ref("INPUT:x", "x"),
                    vec![
                        make_port_ref("gate", "a"),
                        make_port_ref("gate", "b"),
                    ],
                )],
            }],
            components: vec![
                {
                    let mut c = make_component("val", "dff");
                    c.init = Some(true);
                    c
                },
                make_component("inv", "not"),
            ],
            wire_nets: vec![simple_wire_net(
                "w1",
                make_port_ref("val", "out"),
                vec![make_port_ref("inv", "x")],
            )],
        };

        let circuit = project_to_circuit(&project).unwrap();

        // Abstraction's internal component should reference "x" (from INPUT:x).
        assert_eq!(circuit.abstractions.len(), 1);
        assert_eq!(circuit.abstractions[0].components[0].inputs["a"], "x");
        assert_eq!(circuit.abstractions[0].components[0].inputs["b"], "x");

        // Top-level: inv (abstraction instance) has input x=val.
        assert_eq!(circuit.components[1].inputs["x"], "val");
    }

    #[test]
    fn convert_abstraction_instance_source_uses_dot_notation() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 0,
            abstractions: vec![ProjectAbstraction {
                bundle_pins: HashMap::new(),
                pin_positions: HashMap::new(),
                port_layout: HashMap::new(),
                notes: vec![],
            images: vec![],
                name: "not".into(),
                inputs: vec!["x".into()],
                outputs: {
                    let mut m = HashMap::new();
                    m.insert("out".into(), "gate".into());
                    m
                },
                width: 140.0,
                height: 70.0,
                components: vec![make_component("gate", "nand")],
                wire_nets: vec![],
            }],
            components: vec![
                {
                    let mut c = make_component("val", "dff");
                    c.init = Some(true);
                    c
                },
                make_component("inv", "not"),
                make_component("nand_1", "nand"),
            ],
            wire_nets: vec![
                simple_wire_net(
                    "w1",
                    make_port_ref("val", "out"),
                    vec![make_port_ref("inv", "x")],
                ),
                // Wire from abstraction output (dot notation).
                simple_wire_net(
                    "w2",
                    make_port_ref("inv", "out"),
                    vec![
                        make_port_ref("nand_1", "a"),
                        make_port_ref("nand_1", "b"),
                    ],
                ),
            ],
        };

        let circuit = project_to_circuit(&project).unwrap();

        // nand_1 should reference inv.out (dot notation for abstraction output).
        assert_eq!(circuit.components[2].inputs["a"], "inv.out");
        assert_eq!(circuit.components[2].inputs["b"], "inv.out");
    }

    #[test]
    fn convert_unconnected_net() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 0,
            abstractions: vec![],
            components: vec![make_component("clk_1", "clk")],
            wire_nets: vec![WireNet {
                bundle: false,
                id: "orphan".into(),
                source: None,
                sinks: vec![],
                junctions: vec![],
                segments: vec![],
            }],
        };
        let circuit = project_to_circuit(&project).unwrap();
        assert!(circuit.components[0].inputs.is_empty());
    }

    #[test]
    fn convert_net_with_source_no_sinks() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 0,
            abstractions: vec![],
            components: vec![make_component("clk_1", "clk")],
            wire_nets: vec![simple_wire_net(
                "w1",
                make_port_ref("clk_1", "out"),
                vec![],
            )],
        };
        let circuit = project_to_circuit(&project).unwrap();
        assert!(circuit.components[0].inputs.is_empty());
    }

    #[test]
    fn convert_branching_net() {
        let project = Project {
            notes: vec![],
            images: vec![],
            name: "test".into(),
            steps: 0,
            abstractions: vec![],
            components: vec![
                make_component("clk_1", "clk"),
                {
                    let mut c = make_component("dff_1", "dff");
                    c.init = Some(false);
                    c
                },
                {
                    let mut c = make_component("dff_2", "dff");
                    c.init = Some(false);
                    c
                },
            ],
            wire_nets: vec![simple_wire_net(
                "w1",
                make_port_ref("clk_1", "out"),
                vec![
                    make_port_ref("dff_1", "clk"),
                    make_port_ref("dff_2", "clk"),
                ],
            )],
        };

        let circuit = project_to_circuit(&project).unwrap();
        assert_eq!(circuit.components[1].inputs["clk"], "clk_1");
        assert_eq!(circuit.components[2].inputs["clk"], "clk_1");
    }

    #[test]
    fn full_pipeline_toggle_circuit() {
        // Build a toggle flip-flop via Project, convert, expand, compile, simulate.
        use crate::compiler::compile;
        use crate::expander::expand;
        use crate::simulator::simulate;

        let project = Project {
            notes: vec![],
            images: vec![],
            name: "toggle".into(),
            steps: 8,
            abstractions: vec![],
            components: vec![
                make_component("clk", "clk"),
                {
                    let mut c = make_component("state", "dff");
                    c.init = Some(false);
                    c
                },
                make_component("not_state", "nand"),
            ],
            wire_nets: vec![
                simple_wire_net(
                    "w1",
                    make_port_ref("not_state", "out"),
                    vec![make_port_ref("state", "d")],
                ),
                simple_wire_net(
                    "w2",
                    make_port_ref("clk", "out"),
                    vec![make_port_ref("state", "clk")],
                ),
                simple_wire_net(
                    "w3",
                    make_port_ref("state", "out"),
                    vec![
                        make_port_ref("not_state", "a"),
                        make_port_ref("not_state", "b"),
                    ],
                ),
            ],
        };

        let circuit_def = project_to_circuit(&project).unwrap();
        let expanded = expand(&circuit_def).unwrap();
        let compiled = compile(&expanded.def).unwrap();
        let results = simulate(&compiled);

        assert!(!results[0]["clk"]);
        assert!(!results[0]["state"]);
        assert!(results[1]["clk"]);
        assert!(results[1]["state"]);
        assert!(!results[3]["state"]);
    }
}
