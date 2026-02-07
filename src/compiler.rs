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
    pub names: Vec<String>,
    pub components: Vec<CompiledComponent>,
    pub eval_order: Vec<usize>,
    pub clk_idx: Option<usize>,
    pub dff_indices: Vec<usize>,
}

pub fn compile(def: &CircuitDef) -> Result<CompiledCircuit, CompileError> {
    let mut name_to_idx: HashMap<&str, usize> = HashMap::new();
    let mut names = Vec::new();
    let mut clk_count = 0;
    let mut clk_idx = None;
    let mut dff_indices = Vec::new();

    // First pass: register all names.
    for (i, comp) in def.components.iter().enumerate() {
        let name = comp.name();
        if name_to_idx.contains_key(name) {
            return Err(CompileError::DuplicateName(name.to_string()));
        }
        if comp.component_type == "clk" {
            clk_count += 1;
            if clk_count > 1 {
                return Err(CompileError::MultipleClocks);
            }
            clk_idx = Some(i);
        }
        if comp.component_type == "dff" {
            dff_indices.push(i);
        }
        name_to_idx.insert(name, i);
        names.push(name.to_string());
    }

    // Second pass: build compiled components.
    let mut components = Vec::new();
    for comp in &def.components {
        let compiled = build_component(comp, &name_to_idx)?;
        components.push(compiled);
    }

    let eval_order = topological_sort(&components, clk_idx, &dff_indices)?;

    Ok(CompiledCircuit {
        name: def.name.clone(),
        steps: def.steps,
        names,
        components,
        eval_order,
        clk_idx,
        dff_indices,
    })
}

fn build_component(
    comp: &ComponentDef,
    name_to_idx: &HashMap<&str, usize>,
) -> Result<CompiledComponent, CompileError> {
    match comp.component_type.as_str() {
        "clk" => Ok(CompiledComponent::Clk),
        "nand" => {
            let inputs = comp.inputs.as_ref().ok_or_else(|| CompileError::MissingInput {
                component: comp.name.clone(),
                input: "a, b".to_string(),
            })?;
            let a_name = inputs
                .get("a")
                .ok_or_else(|| CompileError::MissingInput {
                    component: comp.name.clone(),
                    input: "a".to_string(),
                })?;
            let b_name = inputs
                .get("b")
                .ok_or_else(|| CompileError::MissingInput {
                    component: comp.name.clone(),
                    input: "b".to_string(),
                })?;
            let a = resolve_input(&comp.name, a_name, name_to_idx)?;
            let b = resolve_input(&comp.name, b_name, name_to_idx)?;
            let self_idx = name_to_idx[comp.name.as_str()];
            if a == self_idx || b == self_idx {
                return Err(CompileError::SelfReference {
                    component: comp.name.clone(),
                });
            }
            Ok(CompiledComponent::Nand {
                input_a: a,
                input_b: b,
            })
        }
        "dff" => {
            let (input_d, input_clk) = match &comp.inputs {
                Some(inputs) => {
                    let d = inputs
                        .get("d")
                        .map(|name| resolve_input(&comp.name, name, name_to_idx))
                        .transpose()?;
                    let clk = inputs
                        .get("clk")
                        .map(|name| resolve_input(&comp.name, name, name_to_idx))
                        .transpose()?;
                    (d, clk)
                }
                None => (None, None),
            };
            Ok(CompiledComponent::Dff {
                init: comp.init.unwrap_or(false),
                input_d,
                input_clk,
            })
        }
        other => Err(CompileError::UnknownType {
            component: comp.name.clone(),
            type_name: other.to_string(),
        }),
    }
}

fn resolve_input(
    component: &str,
    input_name: &str,
    name_to_idx: &HashMap<&str, usize>,
) -> Result<usize, CompileError> {
    name_to_idx
        .get(input_name)
        .copied()
        .ok_or_else(|| CompileError::UnknownInput {
            component: component.to_string(),
            input_name: input_name.to_string(),
        })
}

fn topological_sort(
    components: &[CompiledComponent],
    clk_idx: Option<usize>,
    dff_indices: &[usize],
) -> Result<Vec<usize>, CompileError> {
    let n = components.len();
    let mut in_degree = vec![0usize; n];
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, comp) in components.iter().enumerate() {
        if let CompiledComponent::Nand { input_a, input_b } = comp {
            let a = *input_a;
            let b = *input_b;

            if matches!(components[a], CompiledComponent::Nand { .. }) {
                in_degree[i] += 1;
                dependents[a].push(i);
            }
            if matches!(components[b], CompiledComponent::Nand { .. }) && a != b {
                in_degree[i] += 1;
                dependents[b].push(i);
            }
        }
    }

    let mut eval_order = Vec::with_capacity(n);

    // Sources first: CLK, then DFFs.
    if let Some(idx) = clk_idx {
        eval_order.push(idx);
    }
    for &idx in dff_indices {
        eval_order.push(idx);
    }

    // Kahn's algorithm for NAND gates.
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (i, comp) in components.iter().enumerate() {
        if matches!(comp, CompiledComponent::Nand { .. }) && in_degree[i] == 0 {
            queue.push_back(i);
        }
    }

    let nand_count = components
        .iter()
        .filter(|c| matches!(c, CompiledComponent::Nand { .. }))
        .count();
    let mut sorted_count = 0;

    while let Some(idx) = queue.pop_front() {
        eval_order.push(idx);
        sorted_count += 1;

        for &dep in &dependents[idx] {
            in_degree[dep] -= 1;
            if in_degree[dep] == 0 {
                queue.push_back(dep);
            }
        }
    }

    if sorted_count != nand_count {
        return Err(CompileError::CyclicGraph);
    }

    Ok(eval_order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_yaml(yaml: &str) -> Result<CompiledCircuit, CompileError> {
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        compile(&def)
    }

    #[test]
    fn compile_simple_circuit() {
        let circuit = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: clk
    name: clock
  - type: dff
    name: reg
    init: 0
    inputs:
      d: gate
      clk: clock
  - type: nand
    name: gate
    inputs:
      a: reg
      b: reg
"#,
        )
        .unwrap();

        assert_eq!(circuit.names, vec!["clock", "reg", "gate"]);
        assert_eq!(circuit.clk_idx, Some(0));
        assert_eq!(circuit.dff_indices, vec![1]);
        assert_eq!(circuit.eval_order, vec![0, 1, 2]);
    }

    #[test]
    fn reject_duplicate_names() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: dff
    name: dup
  - type: dff
    name: dup
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::DuplicateName(n) if n == "dup"));
    }

    #[test]
    fn reject_multiple_clocks() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: clk
    name: clk1
  - type: clk
    name: clk2
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::MultipleClocks));
    }

    #[test]
    fn reject_unknown_input() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: nand
    name: gate
    inputs:
      a: nonexistent
      b: nonexistent
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::UnknownInput { .. }));
    }

    #[test]
    fn reject_nand_self_reference() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: dff
    name: x
  - type: nand
    name: bad
    inputs:
      a: bad
      b: x
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::SelfReference { .. }));
    }

    #[test]
    fn reject_cyclic_nand_graph() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: nand
    name: a
    inputs:
      a: b
      b: b
  - type: nand
    name: b
    inputs:
      a: a
      b: a
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::CyclicGraph));
    }

    #[test]
    fn allow_dff_self_reference() {
        let circuit = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: clk
    name: clock
  - type: dff
    name: reg
    init: 1
    inputs:
      d: reg
      clk: clock
"#,
        )
        .unwrap();
        assert_eq!(circuit.names, vec!["clock", "reg"]);
    }

    #[test]
    fn topological_order_chain() {
        let circuit = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: dff
    name: x
    init: 1
  - type: nand
    name: nand3
    inputs: { a: nand2, b: nand2 }
  - type: nand
    name: nand2
    inputs: { a: nand1, b: nand1 }
  - type: nand
    name: nand1
    inputs: { a: x, b: x }
"#,
        )
        .unwrap();

        let nand_order: Vec<&str> = circuit
            .eval_order
            .iter()
            .filter(|&&i| matches!(circuit.components[i], CompiledComponent::Nand { .. }))
            .map(|&i| circuit.names[i].as_str())
            .collect();
        assert_eq!(nand_order, vec!["nand1", "nand2", "nand3"]);
    }

    #[test]
    fn no_clk_is_valid() {
        let circuit = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: dff
    name: a
    init: 1
  - type: nand
    name: not_a
    inputs: { a: a, b: a }
"#,
        )
        .unwrap();
        assert!(circuit.clk_idx.is_none());
    }

    #[test]
    fn reject_three_node_cycle() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: nand
    name: a
    inputs: { a: c, b: c }
  - type: nand
    name: b
    inputs: { a: a, b: a }
  - type: nand
    name: c
    inputs: { a: b, b: b }
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::CyclicGraph));
    }

    #[test]
    fn reject_unknown_type() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: bogus
    name: x
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::UnknownType { .. }));
    }

    #[test]
    fn reject_nand_missing_inputs() {
        let err = compile_yaml(
            r#"
name: test
steps: 1
components:
  - type: nand
    name: x
"#,
        )
        .unwrap_err();
        assert!(matches!(err, CompileError::MissingInput { .. }));
    }
}
