use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use crate::types::*;

#[derive(Debug)]
pub enum ExpandError {
    DuplicateAbstraction(String),
    ClkInAbstraction(String),
    CircularAbstractions,
    SelfReferencingAbstraction(String),
    UnknownType {
        component: String,
        type_name: String,
    },
    UnknownReference {
        context: String,
        reference: String,
    },
    MissingAbstractionInput {
        instance: String,
        abstraction: String,
        input: String,
    },
    ExtraAbstractionInput {
        instance: String,
        abstraction: String,
        input: String,
    },
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateAbstraction(name) => {
                write!(f, "Duplicate abstraction name: \"{name}\"")
            }
            Self::ClkInAbstraction(name) => {
                write!(f, "CLK component not allowed inside abstraction \"{name}\"")
            }
            Self::CircularAbstractions => {
                write!(f, "Circular dependency detected among abstractions")
            }
            Self::SelfReferencingAbstraction(name) => {
                write!(f, "Abstraction \"{name}\" references itself")
            }
            Self::UnknownType {
                component,
                type_name,
            } => write!(
                f,
                "Unknown component type \"{type_name}\" for component \"{component}\""
            ),
            Self::UnknownReference { context, reference } => {
                write!(f, "Unknown reference \"{reference}\" in \"{context}\"")
            }
            Self::MissingAbstractionInput {
                instance,
                abstraction,
                input,
            } => write!(
                f,
                "Instance \"{instance}\" of \"{abstraction}\" is missing input \"{input}\""
            ),
            Self::ExtraAbstractionInput {
                instance,
                abstraction,
                input,
            } => write!(
                f,
                "Instance \"{instance}\" of \"{abstraction}\" has unexpected input \"{input}\""
            ),
        }
    }
}

impl std::error::Error for ExpandError {}

/// Result of expansion: a flat circuit plus output alias mappings.
#[derive(Debug)]
pub struct ExpandedCircuit {
    pub def: CircuitDef,
    /// Maps alias name (e.g. "my_and.out") -> flat component name (e.g. "my_and/inv/gate").
    pub aliases: HashMap<String, String>,
}

/// Expand all abstraction instances into flat primitives.
pub fn expand(circuit_def: &CircuitDef) -> Result<ExpandedCircuit, ExpandError> {
    if circuit_def.abstractions.is_empty() {
        // Still validate that all component types are primitives.
        for component in &circuit_def.components {
            if !is_primitive(&component.component_type) {
                return Err(ExpandError::UnknownType {
                    component: component.name.clone(),
                    type_name: component.component_type.clone(),
                });
            }
        }
        return Ok(ExpandedCircuit {
            def: circuit_def.clone(),
            aliases: HashMap::new(),
        });
    }

    let abstraction_map: HashMap<&str, &AbstractionDef> = circuit_def
        .abstractions
        .iter()
        .map(|abstraction| (abstraction.name.as_str(), abstraction))
        .collect();

    validate_abstractions(&circuit_def.abstractions, &abstraction_map)?;

    let mut flat_components = Vec::new();
    let mut aliases: HashMap<String, String> = HashMap::new();

    for component in &circuit_def.components {
        if is_primitive(&component.component_type) {
            flat_components.push(component.clone());
        } else if let Some(abstraction) =
            abstraction_map.get(component.component_type.as_str())
        {
            let input_bindings = component.inputs.clone();

            validate_instance_inputs(&component.name, abstraction, &input_bindings)?;

            let output_mapping = expand_instance(
                &component.name,
                abstraction,
                &input_bindings,
                &abstraction_map,
                &mut flat_components,
            )?;

            for (output_name, flat_target_name) in output_mapping {
                aliases.insert(
                    format!("{}.{}", component.name, output_name),
                    flat_target_name,
                );
            }
        } else {
            return Err(ExpandError::UnknownType {
                component: component.name.clone(),
                type_name: component.component_type.clone(),
            });
        }
    }

    // Resolve dot-notation references in all flat components.
    for component in &mut flat_components {
        for input_value in component.inputs.values_mut() {
            if let Some(resolved) = aliases.get(input_value.as_str()) {
                *input_value = resolved.clone();
            }
        }
    }

    Ok(ExpandedCircuit {
        def: CircuitDef {
            name: circuit_def.name.clone(),
            steps: circuit_def.steps,
            abstractions: vec![],
            components: flat_components,
        },
        aliases,
    })
}

fn validate_abstractions(
    abstractions: &[AbstractionDef],
    abstraction_map: &HashMap<&str, &AbstractionDef>,
) -> Result<(), ExpandError> {
    // Check for duplicate names.
    let mut seen_names = HashSet::new();
    for abstraction in abstractions {
        if !seen_names.insert(&abstraction.name) {
            return Err(ExpandError::DuplicateAbstraction(abstraction.name.clone()));
        }
    }

    // Check for CLK inside abstractions.
    for abstraction in abstractions {
        check_no_clk_in_abstraction(abstraction)?;
    }

    // Check for self-references and circular dependencies.
    check_abstraction_cycles(abstractions, abstraction_map)?;

    Ok(())
}

/// Check that no CLK component exists directly inside an abstraction.
fn check_no_clk_in_abstraction(abstraction: &AbstractionDef) -> Result<(), ExpandError> {
    for component in &abstraction.components {
        if component.component_type == "clk" {
            return Err(ExpandError::ClkInAbstraction(abstraction.name.clone()));
        }
    }
    Ok(())
}

/// Verify that the abstraction dependency graph is acyclic using Kahn's algorithm.
fn check_abstraction_cycles(
    abstractions: &[AbstractionDef],
    abstraction_map: &HashMap<&str, &AbstractionDef>,
) -> Result<(), ExpandError> {
    let abstraction_names: HashSet<&str> = abstraction_map.keys().copied().collect();

    // Build dependency graph: abstraction -> set of abstractions it uses.
    let mut incoming_dependency_count: HashMap<&str, usize> = HashMap::new();
    let mut downstream_dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for abstraction in abstractions {
        incoming_dependency_count
            .entry(abstraction.name.as_str())
            .or_insert(0);
        downstream_dependents
            .entry(abstraction.name.as_str())
            .or_default();

        for component in &abstraction.components {
            if abstraction_names.contains(component.component_type.as_str()) {
                if component.component_type == abstraction.name {
                    return Err(ExpandError::SelfReferencingAbstraction(
                        abstraction.name.clone(),
                    ));
                }
                *incoming_dependency_count
                    .entry(abstraction.name.as_str())
                    .or_insert(0) += 1;
                downstream_dependents
                    .entry(component.component_type.as_str())
                    .or_default()
                    .push(abstraction.name.as_str());
            }
        }
    }

    // Kahn's algorithm to detect cycles.
    let mut ready_queue: VecDeque<&str> = incoming_dependency_count
        .iter()
        .filter(|(_, &count)| count == 0)
        .map(|(&name, _)| name)
        .collect();

    let mut sorted_count = 0;
    while let Some(current_name) = ready_queue.pop_front() {
        sorted_count += 1;
        if let Some(dependents) = downstream_dependents.get(current_name) {
            for &dependent_name in dependents {
                let count = incoming_dependency_count.get_mut(dependent_name).unwrap();
                *count -= 1;
                if *count == 0 {
                    ready_queue.push_back(dependent_name);
                }
            }
        }
    }

    if sorted_count != abstractions.len() {
        return Err(ExpandError::CircularAbstractions);
    }

    Ok(())
}

/// Validate that an instance provides exactly the inputs declared by its abstraction.
fn validate_instance_inputs(
    instance_name: &str,
    abstraction: &AbstractionDef,
    input_bindings: &HashMap<String, String>,
) -> Result<(), ExpandError> {
    // Check all declared inputs are provided.
    for declared_input in &abstraction.inputs {
        if !input_bindings.contains_key(declared_input) {
            return Err(ExpandError::MissingAbstractionInput {
                instance: instance_name.to_string(),
                abstraction: abstraction.name.clone(),
                input: declared_input.clone(),
            });
        }
    }

    // Check no extra inputs are provided.
    let declared_input_names: HashSet<&str> =
        abstraction.inputs.iter().map(|s| s.as_str()).collect();
    for provided_key in input_bindings.keys() {
        if !declared_input_names.contains(provided_key.as_str()) {
            return Err(ExpandError::ExtraAbstractionInput {
                instance: instance_name.to_string(),
                abstraction: abstraction.name.clone(),
                input: provided_key.clone(),
            });
        }
    }

    Ok(())
}

/// Expand one abstraction instance into flat primitives.
/// Returns a mapping of output_name -> flat_component_name for the instance's outputs.
fn expand_instance(
    name_prefix: &str,
    abstraction: &AbstractionDef,
    input_bindings: &HashMap<String, String>,
    abstraction_map: &HashMap<&str, &AbstractionDef>,
    flat_components: &mut Vec<ComponentDef>,
) -> Result<HashMap<String, String>, ExpandError> {
    // Phase 1: Register all internal component flat names.
    let mut internal_name_map: HashMap<String, String> = HashMap::new();
    for component in &abstraction.components {
        internal_name_map.insert(
            component.name.clone(),
            format!("{name_prefix}/{}", component.name),
        );
    }

    // Phase 2: Expand sub-abstraction instances (in definition order)
    // to learn their output aliases.
    for component in &abstraction.components {
        if !is_primitive(&component.component_type) {
            let sub_abstraction =
                abstraction_map
                    .get(component.component_type.as_str())
                    .ok_or_else(|| ExpandError::UnknownType {
                        component: format!("{name_prefix}/{}", component.name),
                        type_name: component.component_type.clone(),
                    })?;

            let sub_input_bindings = resolve_input_map(
                &format!("{name_prefix}/{}", component.name),
                &component.inputs,
                input_bindings,
                &internal_name_map,
            )?;

            validate_instance_inputs(
                &format!("{name_prefix}/{}", component.name),
                sub_abstraction,
                &sub_input_bindings,
            )?;

            let sub_output_mapping = expand_instance(
                &format!("{name_prefix}/{}", component.name),
                sub_abstraction,
                &sub_input_bindings,
                abstraction_map,
                flat_components,
            )?;

            // Register sub-output aliases: "comp_name.out_name" -> flat name.
            for (output_name, flat_target_name) in sub_output_mapping {
                internal_name_map.insert(
                    format!("{}.{}", component.name, output_name),
                    flat_target_name,
                );
            }
        }
    }

    // Phase 3: Add primitive components with resolved inputs.
    for component in &abstraction.components {
        if is_primitive(&component.component_type) {
            let flat_name = format!("{name_prefix}/{}", component.name);
            let resolved_inputs = resolve_input_map(
                &flat_name,
                &component.inputs,
                input_bindings,
                &internal_name_map,
            )?;

            flat_components.push(ComponentDef {
                component_type: component.component_type.clone(),
                name: flat_name,
                init: component.init,
                inputs: resolved_inputs,
            });
        }
    }

    // Phase 4: Resolve the abstraction's output mapping.
    let mut output_mapping = HashMap::new();
    for (output_name, internal_reference) in &abstraction.outputs {
        let resolved = resolve_reference(
            &format!("{name_prefix} output \"{output_name}\""),
            internal_reference,
            input_bindings,
            &internal_name_map,
        )?;
        output_mapping.insert(output_name.clone(), resolved);
    }

    Ok(output_mapping)
}

/// Resolve all inputs of a component, mapping each reference to its flat name.
fn resolve_input_map(
    context: &str,
    inputs: &HashMap<String, String>,
    input_bindings: &HashMap<String, String>,
    internal_name_map: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ExpandError> {
    inputs
        .iter()
        .map(|(input_key, input_reference)| {
            let resolved =
                resolve_reference(context, input_reference, input_bindings, internal_name_map)?;
            Ok((input_key.clone(), resolved))
        })
        .collect()
}

/// Resolve a single reference: first check abstraction input parameters,
/// then check internal component names and sub-output aliases.
fn resolve_reference(
    context: &str,
    reference: &str,
    input_bindings: &HashMap<String, String>,
    internal_name_map: &HashMap<String, String>,
) -> Result<String, ExpandError> {
    // First: is it an abstraction input parameter?
    if let Some(resolved) = input_bindings.get(reference) {
        return Ok(resolved.clone());
    }
    // Second: is it an internal component or sub-output alias?
    if let Some(resolved) = internal_name_map.get(reference) {
        return Ok(resolved.clone());
    }
    Err(ExpandError::UnknownReference {
        context: context.to_string(),
        reference: reference.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{AbstractionBuilder, CircuitBuilder};

    fn not_abstraction() -> AbstractionDef {
        AbstractionBuilder::new("not")
            .inputs(&["x"])
            .output("out", "gate")
            .nand("gate", "x", "x")
            .build()
    }

    #[test]
    fn reject_clk_in_abstraction() {
        let bad_abs = AbstractionDef {
            name: "bad".into(),
            inputs: vec![],
            outputs: HashMap::new(),
            components: vec![ComponentDef {
                component_type: "clk".into(),
                name: "clock".into(),
                init: None,
                inputs: HashMap::new(),
            }],
        };
        let def = CircuitBuilder::new("test", 1)
            .abstraction(bad_abs)
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::ClkInAbstraction(n) if n == "bad"));
    }

    #[test]
    fn reject_self_referencing_abstraction() {
        let bad_abs = AbstractionBuilder::new("bad")
            .inputs(&["x"])
            .output("out", "sub.out")
            .component("bad", "sub", &[("x", "x")])
            .build();
        let def = CircuitBuilder::new("test", 1)
            .abstraction(bad_abs)
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::SelfReferencingAbstraction(n) if n == "bad"));
    }

    #[test]
    fn reject_circular_abstractions() {
        let abs_a = AbstractionBuilder::new("a")
            .inputs(&["x"])
            .output("out", "sub.out")
            .component("b", "sub", &[("x", "x")])
            .build();
        let abs_b = AbstractionBuilder::new("b")
            .inputs(&["x"])
            .output("out", "sub.out")
            .component("a", "sub", &[("x", "x")])
            .build();
        let def = CircuitBuilder::new("test", 1)
            .abstraction(abs_a)
            .abstraction(abs_b)
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::CircularAbstractions));
    }

    #[test]
    fn reject_duplicate_abstraction_names() {
        let abs1 = AbstractionBuilder::new("dup")
            .inputs(&[])
            .build();
        let abs2 = AbstractionBuilder::new("dup")
            .inputs(&[])
            .build();
        let def = CircuitBuilder::new("test", 1)
            .abstraction(abs1)
            .abstraction(abs2)
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::DuplicateAbstraction(n) if n == "dup"));
    }

    #[test]
    fn reject_unknown_type() {
        let def = CircuitBuilder::new("test", 1)
            .component("nonexistent", "bad", &[("x", "y")])
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::UnknownType { .. }));
    }

    #[test]
    fn reject_missing_abstraction_input() {
        let def = CircuitBuilder::new("test", 1)
            .abstraction(not_abstraction())
            .dff("a").done()
            .component("not", "my_not", &[])
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::MissingAbstractionInput { .. }));
    }

    #[test]
    fn reject_extra_abstraction_input() {
        let def = CircuitBuilder::new("test", 1)
            .abstraction(not_abstraction())
            .dff("a").done()
            .component("not", "my_not", &[("x", "a"), ("y", "a")])
            .build();
        let err = expand(&def).unwrap_err();
        assert!(matches!(err, ExpandError::ExtraAbstractionInput { .. }));
    }

    #[test]
    fn expand_simple_not() {
        let def = CircuitBuilder::new("test", 1)
            .abstraction(not_abstraction())
            .dff("a").init(true).done()
            .component("not", "inv", &[("x", "a")])
            .build();
        let expanded = expand(&def).unwrap();
        assert_eq!(expanded.def.components.len(), 2); // dff + expanded nand
        let nand = &expanded.def.components[1];
        assert_eq!(nand.name, "inv/gate");
        assert_eq!(nand.component_type, "nand");
        assert_eq!(nand.inputs["a"], "a");
        assert_eq!(nand.inputs["b"], "a");
        assert_eq!(expanded.aliases["inv.out"], "inv/gate");
    }

    #[test]
    fn expand_nested_and() {
        let and_abs = AbstractionBuilder::new("and")
            .inputs(&["a", "b"])
            .output("out", "inv.out")
            .nand("nand_ab", "a", "b")
            .component("not", "inv", &[("x", "nand_ab")])
            .build();
        let def = CircuitBuilder::new("test", 1)
            .abstraction(not_abstraction())
            .abstraction(and_abs)
            .dff("x").init(true).done()
            .dff("y").init(true).done()
            .component("and", "my_and", &[("a", "x"), ("b", "y")])
            .build();
        let expanded = expand(&def).unwrap();
        // dff x, dff y, nand my_and/nand_ab, nand my_and/inv/gate
        assert_eq!(expanded.def.components.len(), 4);

        let names: Vec<&str> = expanded.def.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"my_and/nand_ab"));
        assert!(names.contains(&"my_and/inv/gate"));
        assert_eq!(expanded.aliases["my_and.out"], "my_and/inv/gate");
    }

    #[test]
    fn expand_preserves_init() {
        let reg_abs = AbstractionBuilder::new("reg")
            .inputs(&["d", "clk"])
            .output("q", "ff")
            .dff("ff").init(true).d("d").clk("clk").done()
            .build();
        let def = CircuitBuilder::new("test", 1)
            .abstraction(reg_abs)
            .clk("clock")
            .dff("data").init(true).done()
            .component("reg", "my_reg", &[("d", "data"), ("clk", "clock")])
            .build();
        let expanded = expand(&def).unwrap();
        let flip_flop = expanded.def.components.iter().find(|c| c.name == "my_reg/ff").unwrap();
        assert_eq!(flip_flop.init, Some(true));
    }

    #[test]
    fn no_abstractions_passthrough() {
        let def = CircuitBuilder::new("test", 1)
            .dff("a").init(true).done()
            .nand("b", "a", "a")
            .build();
        let expanded = expand(&def).unwrap();
        assert_eq!(expanded.def.components.len(), 2);
        assert!(expanded.aliases.is_empty());
    }

    #[test]
    fn expand_with_dot_reference_in_top_level() {
        let def = CircuitBuilder::new("test", 1)
            .abstraction(not_abstraction())
            .dff("a").init(true).done()
            .component("not", "inv", &[("x", "a")])
            .nand("result", "inv.out", "inv.out")
            .build();
        let expanded = expand(&def).unwrap();
        let result = expanded.def.components.iter().find(|c| c.name == "result").unwrap();
        assert_eq!(result.inputs["a"], "inv/gate");
        assert_eq!(result.inputs["b"], "inv/gate");
    }

    #[test]
    fn expand_three_levels_deep() {
        let and_abs = AbstractionBuilder::new("and")
            .inputs(&["a", "b"])
            .output("out", "inv.out")
            .nand("nab", "a", "b")
            .component("not", "inv", &[("x", "nab")])
            .build();
        let or_abs = AbstractionBuilder::new("or")
            .inputs(&["a", "b"])
            .output("out", "nand_notab")
            .component("not", "na", &[("x", "a")])
            .component("not", "nb", &[("x", "b")])
            .nand("nand_notab", "na.out", "nb.out")
            .build();
        let def = CircuitBuilder::new("test", 0)
            .abstraction(not_abstraction())
            .abstraction(and_abs)
            .abstraction(or_abs)
            .dff("x").init(true).done()
            .dff("y").init(false).done()
            .component("or", "my_or", &[("a", "x"), ("b", "y")])
            .build();
        let expanded = expand(&def).unwrap();
        // dff x, dff y, nand my_or/na/gate, nand my_or/nb/gate, nand my_or/nand_notab
        assert_eq!(expanded.def.components.len(), 5);
        assert_eq!(expanded.aliases["my_or.out"], "my_or/nand_notab");
    }

    #[test]
    fn abstraction_with_no_inputs() {
        let const_one = AbstractionBuilder::new("const_one")
            .inputs(&[])
            .output("out", "val")
            .dff("val").init(true).done()
            .build();
        let def = CircuitBuilder::new("test", 0)
            .abstraction(const_one)
            .component("const_one", "one", &[])
            .build();
        let expanded = expand(&def).unwrap();
        assert_eq!(expanded.def.components.len(), 1);
        assert_eq!(expanded.def.components[0].name, "one/val");
        assert_eq!(expanded.aliases["one.out"], "one/val");
    }
}
