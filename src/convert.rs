use std::collections::HashMap;
use std::fmt;

use crate::project::{PortRef, Project, ProjectAbstraction, ProjectComponent, WireNet};
use crate::types::{is_primitive, AbstractionDef, CircuitDef, ComponentDef};

#[derive(Debug)]
pub enum ConvertError {
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
    let abstractions = project
        .abstractions
        .iter()
        .map(|pa| convert_abstraction(pa))
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
        outputs: pa.outputs.clone(),
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::*;

    fn make_component(id: &str, comp_type: &str) -> ProjectComponent {
        ProjectComponent {
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
            name: "test".into(),
            steps: 0,
            abstractions: vec![ProjectAbstraction {
                pin_positions: HashMap::new(),
                port_layout: HashMap::new(),
                notes: vec![],
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
            name: "test".into(),
            steps: 0,
            abstractions: vec![ProjectAbstraction {
                pin_positions: HashMap::new(),
                port_layout: HashMap::new(),
                notes: vec![],
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
            name: "test".into(),
            steps: 0,
            abstractions: vec![],
            components: vec![make_component("clk_1", "clk")],
            wire_nets: vec![WireNet {
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

        assert_eq!(results[0]["clk"], false);
        assert_eq!(results[0]["state"], false);
        assert_eq!(results[1]["clk"], true);
        assert_eq!(results[1]["state"], true);
        assert_eq!(results[3]["state"], false);
    }
}
