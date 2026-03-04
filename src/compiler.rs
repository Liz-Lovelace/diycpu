use std::collections::{HashMap, VecDeque};
use std::fmt;

use crate::types::*;

#[derive(Debug)]
pub enum CompileError {
    DuplicateName(String),
    MultipleClocks,
    UnknownInput {
        component: String,
        input_name: String,
    },
    CyclicGraph,
    SelfReference {
        component: String,
    },
    UnknownType {
        component: String,
        type_name: String,
    },
    MissingInput {
        component: String,
        input: String,
    },
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(f, "Duplicate component name: \"{name}\""),
            Self::MultipleClocks => {
                write!(f, "Multiple CLK components defined (at most one allowed)")
            }
            Self::UnknownInput {
                component,
                input_name,
            } => write!(f, "Unknown input \"{input_name}\" in component \"{component}\""),
            Self::CyclicGraph => write!(f, "Cyclic dependency detected among NAND gates"),
            Self::SelfReference { component } => {
                write!(f, "NAND gate \"{component}\" references itself as input")
            }
            Self::UnknownType {
                component,
                type_name,
            } => write!(
                f,
                "Unknown primitive type \"{type_name}\" for component \"{component}\""
            ),
            Self::MissingInput { component, input } => {
                write!(f, "Missing input \"{input}\" in component \"{component}\"")
            }
        }
    }
}

impl std::error::Error for CompileError {}

#[derive(Debug)]
pub enum CompiledComponent {
    Clk,
    Nand {
        input_a: usize,
        input_b: usize,
    },
    Dff {
        init: bool,
        input_d: Option<usize>,
        input_clk: Option<usize>,
    },
}

#[derive(Debug)]
pub struct CompiledCircuit {
    pub name: String,
    pub steps: usize,
    pub component_names: Vec<String>,
    pub components: Vec<CompiledComponent>,
    pub evaluation_order: Vec<usize>,
    pub clock_index: Option<usize>,
    pub dff_indices: Vec<usize>,
}

pub fn compile(circuit_def: &CircuitDef) -> Result<CompiledCircuit, CompileError> {
    let mut name_to_index: HashMap<&str, usize> = HashMap::new();
    let mut component_names = Vec::new();
    let mut clock_count = 0;
    let mut clock_index = None;
    let mut dff_indices = Vec::new();

    // First pass: register all component names and validate uniqueness.
    for (component_index, component) in circuit_def.components.iter().enumerate() {
        let component_name = component.name();
        if name_to_index.contains_key(component_name) {
            return Err(CompileError::DuplicateName(component_name.to_string()));
        }
        if component.component_type == "clk" {
            clock_count += 1;
            if clock_count > 1 {
                return Err(CompileError::MultipleClocks);
            }
            clock_index = Some(component_index);
        }
        if component.component_type == "dff" {
            dff_indices.push(component_index);
        }
        name_to_index.insert(component_name, component_index);
        component_names.push(component_name.to_string());
    }

    // Second pass: build compiled components with resolved input indices.
    let mut compiled_components = Vec::new();
    for component in &circuit_def.components {
        let compiled = build_compiled_component(component, &name_to_index)?;
        compiled_components.push(compiled);
    }

    let evaluation_order =
        topological_sort(&compiled_components, clock_index, &dff_indices)?;

    Ok(CompiledCircuit {
        name: circuit_def.name.clone(),
        steps: circuit_def.steps,
        component_names,
        components: compiled_components,
        evaluation_order,
        clock_index,
        dff_indices,
    })
}

fn build_compiled_component(
    component: &ComponentDef,
    name_to_index: &HashMap<&str, usize>,
) -> Result<CompiledComponent, CompileError> {
    match component.component_type.as_str() {
        "clk" => Ok(CompiledComponent::Clk),
        "nand" => {
            let input_a_name =
                component.inputs
                    .get("a")
                    .ok_or_else(|| CompileError::MissingInput {
                        component: component.name.clone(),
                        input: "a".to_string(),
                    })?;
            let input_b_name =
                component.inputs
                    .get("b")
                    .ok_or_else(|| CompileError::MissingInput {
                        component: component.name.clone(),
                        input: "b".to_string(),
                    })?;

            let input_a_index =
                resolve_input_reference(&component.name, input_a_name, name_to_index)?;
            let input_b_index =
                resolve_input_reference(&component.name, input_b_name, name_to_index)?;

            let own_index = name_to_index[component.name.as_str()];
            if input_a_index == own_index || input_b_index == own_index {
                return Err(CompileError::SelfReference {
                    component: component.name.clone(),
                });
            }

            Ok(CompiledComponent::Nand {
                input_a: input_a_index,
                input_b: input_b_index,
            })
        }
        "dff" => {
            let input_d = component.inputs
                .get("d")
                .map(|name| resolve_input_reference(&component.name, name, name_to_index))
                .transpose()?;
            let input_clk = component.inputs
                .get("clk")
                .map(|name| resolve_input_reference(&component.name, name, name_to_index))
                .transpose()?;
            Ok(CompiledComponent::Dff {
                init: component.init.unwrap_or(false),
                input_d,
                input_clk,
            })
        }
        unknown_type => Err(CompileError::UnknownType {
            component: component.name.clone(),
            type_name: unknown_type.to_string(),
        }),
    }
}

/// Look up a component name and return its index. Returns an error if the name
/// doesn't match any registered component.
fn resolve_input_reference(
    component_name: &str,
    input_name: &str,
    name_to_index: &HashMap<&str, usize>,
) -> Result<usize, CompileError> {
    name_to_index
        .get(input_name)
        .copied()
        .ok_or_else(|| CompileError::UnknownInput {
            component: component_name.to_string(),
            input_name: input_name.to_string(),
        })
}

/// Compute the evaluation order using Kahn's topological sort algorithm.
/// CLK and DFF components are evaluated first (they provide stable values).
/// NAND gates are then sorted by their dependency graph.
fn topological_sort(
    components: &[CompiledComponent],
    clock_index: Option<usize>,
    dff_indices: &[usize],
) -> Result<Vec<usize>, CompileError> {
    let component_count = components.len();
    let mut incoming_edge_count = vec![0usize; component_count];
    let mut downstream_dependents: Vec<Vec<usize>> = vec![Vec::new(); component_count];

    // Build the dependency graph among NAND gates only.
    for (component_index, component) in components.iter().enumerate() {
        if let CompiledComponent::Nand { input_a, input_b } = component {
            let source_a = *input_a;
            let source_b = *input_b;

            if matches!(components[source_a], CompiledComponent::Nand { .. }) {
                incoming_edge_count[component_index] += 1;
                downstream_dependents[source_a].push(component_index);
            }
            if matches!(components[source_b], CompiledComponent::Nand { .. }) && source_a != source_b
            {
                incoming_edge_count[component_index] += 1;
                downstream_dependents[source_b].push(component_index);
            }
        }
    }

    let mut evaluation_order = Vec::with_capacity(component_count);

    // Sources first: CLK, then DFFs (they don't depend on NAND gates).
    if let Some(index) = clock_index {
        evaluation_order.push(index);
    }
    for &index in dff_indices {
        evaluation_order.push(index);
    }

    // Kahn's algorithm: start with NAND gates that have no NAND inputs.
    let mut ready_queue: VecDeque<usize> = VecDeque::new();
    for (component_index, component) in components.iter().enumerate() {
        if matches!(component, CompiledComponent::Nand { .. })
            && incoming_edge_count[component_index] == 0
        {
            ready_queue.push_back(component_index);
        }
    }

    let total_nand_count = components
        .iter()
        .filter(|component| matches!(component, CompiledComponent::Nand { .. }))
        .count();
    let mut sorted_nand_count = 0;

    while let Some(current_index) = ready_queue.pop_front() {
        evaluation_order.push(current_index);
        sorted_nand_count += 1;

        for &dependent_index in &downstream_dependents[current_index] {
            incoming_edge_count[dependent_index] -= 1;
            if incoming_edge_count[dependent_index] == 0 {
                ready_queue.push_back(dependent_index);
            }
        }
    }

    if sorted_nand_count != total_nand_count {
        return Err(CompileError::CyclicGraph);
    }

    Ok(evaluation_order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::CircuitBuilder;

    #[test]
    fn compile_simple_circuit() {
        let def = CircuitBuilder::new("test", 1)
            .clk("clock")
            .dff("reg").init(false).d("gate").clk("clock").done()
            .nand("gate", "reg", "reg")
            .build();
        let circuit = compile(&def).unwrap();

        assert_eq!(circuit.component_names, vec!["clock", "reg", "gate"]);
        assert_eq!(circuit.clock_index, Some(0));
        assert_eq!(circuit.dff_indices, vec![1]);
        assert_eq!(circuit.evaluation_order, vec![0, 1, 2]);
    }

    #[test]
    fn reject_duplicate_names() {
        let def = CircuitBuilder::new("test", 1)
            .dff("dup").done()
            .dff("dup").done()
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::DuplicateName(n) if n == "dup"));
    }

    #[test]
    fn reject_multiple_clocks() {
        let def = CircuitBuilder::new("test", 1)
            .clk("clk1")
            .clk("clk2")
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::MultipleClocks));
    }

    #[test]
    fn reject_unknown_input() {
        let def = CircuitBuilder::new("test", 1)
            .nand("gate", "nonexistent", "nonexistent")
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::UnknownInput { .. }));
    }

    #[test]
    fn reject_nand_self_reference() {
        let def = CircuitBuilder::new("test", 1)
            .dff("x").done()
            .nand("bad", "bad", "x")
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::SelfReference { .. }));
    }

    #[test]
    fn reject_cyclic_nand_graph() {
        let def = CircuitBuilder::new("test", 1)
            .nand("a", "b", "b")
            .nand("b", "a", "a")
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::CyclicGraph));
    }

    #[test]
    fn allow_dff_self_reference() {
        let def = CircuitBuilder::new("test", 1)
            .clk("clock")
            .dff("reg").init(true).d("reg").clk("clock").done()
            .build();
        let circuit = compile(&def).unwrap();
        assert_eq!(circuit.component_names, vec!["clock", "reg"]);
    }

    #[test]
    fn topological_order_chain() {
        let def = CircuitBuilder::new("test", 1)
            .dff("x").init(true).done()
            .nand("nand3", "nand2", "nand2")
            .nand("nand2", "nand1", "nand1")
            .nand("nand1", "x", "x")
            .build();
        let circuit = compile(&def).unwrap();

        let nand_order: Vec<&str> = circuit
            .evaluation_order
            .iter()
            .filter(|&&i| matches!(circuit.components[i], CompiledComponent::Nand { .. }))
            .map(|&i| circuit.component_names[i].as_str())
            .collect();
        assert_eq!(nand_order, vec!["nand1", "nand2", "nand3"]);
    }

    #[test]
    fn no_clk_is_valid() {
        let def = CircuitBuilder::new("test", 1)
            .dff("a").init(true).done()
            .nand("not_a", "a", "a")
            .build();
        let circuit = compile(&def).unwrap();
        assert!(circuit.clock_index.is_none());
    }

    #[test]
    fn reject_three_node_cycle() {
        let def = CircuitBuilder::new("test", 1)
            .nand("a", "c", "c")
            .nand("b", "a", "a")
            .nand("c", "b", "b")
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::CyclicGraph));
    }

    #[test]
    fn reject_unknown_type() {
        let def = CircuitBuilder::new("test", 1)
            .component("bogus", "x", &[])
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::UnknownType { .. }));
    }

    #[test]
    fn reject_nand_missing_inputs() {
        let def = CircuitBuilder::new("test", 1)
            .component("nand", "x", &[])
            .build();
        let err = compile(&def).unwrap_err();
        assert!(matches!(err, CompileError::MissingInput { .. }));
    }
}
