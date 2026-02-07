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
            let input_bindings = component.inputs.as_ref().cloned().unwrap_or_default();

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
        if let Some(inputs) = &mut component.inputs {
            for input_value in inputs.values_mut() {
                if let Some(resolved) = aliases.get(input_value.as_str()) {
                    *input_value = resolved.clone();
                }
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
                component.inputs.as_ref(),
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
                component.inputs.as_ref(),
                input_bindings,
                &internal_name_map,
            )?;

            flat_components.push(ComponentDef {
                component_type: component.component_type.clone(),
                name: flat_name,
                init: component.init,
                inputs: if resolved_inputs.is_empty() {
                    None
                } else {
                    Some(resolved_inputs)
                },
                pos: None,
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
    inputs: Option<&HashMap<String, String>>,
    input_bindings: &HashMap<String, String>,
    internal_name_map: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ExpandError> {
    let Some(inputs) = inputs else {
        return Ok(HashMap::new());
    };
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

    fn parse(yaml: &str) -> CircuitDef {
        serde_yaml::from_str(yaml).unwrap()
    }

    fn expand_ok(yaml: &str) -> ExpandedCircuit {
        let def = parse(yaml);
        expand(&def).unwrap()
    }

    fn expand_err(yaml: &str) -> ExpandError {
        let def = parse(yaml);
        expand(&def).unwrap_err()
    }

    #[test]
    fn reject_clk_in_abstraction() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions:
  - name: bad
    inputs: []
    outputs: {}
    components:
      - type: clk
        name: clock
components: []
"#,
        );
        assert!(matches!(err, ExpandError::ClkInAbstraction(n) if n == "bad"));
    }

    #[test]
    fn reject_self_referencing_abstraction() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions:
  - name: bad
    inputs: [x]
    outputs: { out: sub.out }
    components:
      - type: bad
        name: sub
        inputs: { x: x }
components: []
"#,
        );
        assert!(matches!(err, ExpandError::SelfReferencingAbstraction(n) if n == "bad"));
    }

    #[test]
    fn reject_circular_abstractions() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions:
  - name: a
    inputs: [x]
    outputs: { out: sub.out }
    components:
      - type: b
        name: sub
        inputs: { x: x }
  - name: b
    inputs: [x]
    outputs: { out: sub.out }
    components:
      - type: a
        name: sub
        inputs: { x: x }
components: []
"#,
        );
        assert!(matches!(err, ExpandError::CircularAbstractions));
    }

    #[test]
    fn reject_duplicate_abstraction_names() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions:
  - name: dup
    inputs: []
    outputs: {}
    components: []
  - name: dup
    inputs: []
    outputs: {}
    components: []
components: []
"#,
        );
        assert!(matches!(err, ExpandError::DuplicateAbstraction(n) if n == "dup"));
    }

    #[test]
    fn reject_unknown_type() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions: []
components:
  - type: nonexistent
    name: bad
    inputs: { x: y }
"#,
        );
        assert!(matches!(err, ExpandError::UnknownType { .. }));
    }

    #[test]
    fn reject_missing_abstraction_input() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions:
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
components:
  - type: dff
    name: a
  - type: not
    name: my_not
    inputs: {}
"#,
        );
        assert!(matches!(err, ExpandError::MissingAbstractionInput { .. }));
    }

    #[test]
    fn reject_extra_abstraction_input() {
        let err = expand_err(
            r#"
name: test
steps: 1
abstractions:
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
components:
  - type: dff
    name: a
  - type: not
    name: my_not
    inputs: { x: a, y: a }
"#,
        );
        assert!(matches!(err, ExpandError::ExtraAbstractionInput { .. }));
    }

    #[test]
    fn expand_simple_not() {
        let expanded = expand_ok(
            r#"
name: test
steps: 1
abstractions:
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
components:
  - type: dff
    name: a
    init: true
  - type: not
    name: inv
    inputs: { x: a }
"#,
        );
        assert_eq!(expanded.def.components.len(), 2); // dff + expanded nand
        let nand = &expanded.def.components[1];
        assert_eq!(nand.name, "inv/gate");
        assert_eq!(nand.component_type, "nand");
        let inputs = nand.inputs.as_ref().unwrap();
        assert_eq!(inputs["a"], "a");
        assert_eq!(inputs["b"], "a");
        assert_eq!(expanded.aliases["inv.out"], "inv/gate");
    }

    #[test]
    fn expand_nested_and() {
        let expanded = expand_ok(
            r#"
name: test
steps: 1
abstractions:
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
  - name: and
    inputs: [a, b]
    outputs: { out: inv.out }
    components:
      - type: nand
        name: nand_ab
        inputs: { a: a, b: b }
      - type: not
        name: inv
        inputs: { x: nand_ab }
components:
  - type: dff
    name: x
    init: true
  - type: dff
    name: y
    init: true
  - type: and
    name: my_and
    inputs: { a: x, b: y }
"#,
        );
        // Should have: dff x, dff y, nand my_and/nand_ab, nand my_and/inv/gate
        assert_eq!(expanded.def.components.len(), 4);

        let names: Vec<&str> = expanded
            .def
            .components
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(names.contains(&"my_and/nand_ab"));
        assert!(names.contains(&"my_and/inv/gate"));

        // Check alias resolution
        assert_eq!(expanded.aliases["my_and.out"], "my_and/inv/gate");
    }

    #[test]
    fn expand_preserves_init() {
        let expanded = expand_ok(
            r#"
name: test
steps: 1
abstractions:
  - name: reg
    inputs: [d, clk]
    outputs: { q: ff }
    components:
      - type: dff
        name: ff
        init: true
        inputs: { d: d, clk: clk }
components:
  - type: clk
    name: clock
  - type: dff
    name: data
    init: true
  - type: reg
    name: my_reg
    inputs: { d: data, clk: clock }
"#,
        );
        let flip_flop = expanded
            .def
            .components
            .iter()
            .find(|c| c.name == "my_reg/ff")
            .unwrap();
        assert_eq!(flip_flop.init, Some(true));
    }

    #[test]
    fn no_abstractions_passthrough() {
        let expanded = expand_ok(
            r#"
name: test
steps: 1
components:
  - type: dff
    name: a
    init: true
  - type: nand
    name: b
    inputs: { a: a, b: a }
"#,
        );
        assert_eq!(expanded.def.components.len(), 2);
        assert!(expanded.aliases.is_empty());
    }

    #[test]
    fn expand_with_dot_reference_in_top_level() {
        let expanded = expand_ok(
            r#"
name: test
steps: 1
abstractions:
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
components:
  - type: dff
    name: a
    init: true
  - type: not
    name: inv
    inputs: { x: a }
  - type: nand
    name: result
    inputs: { a: inv.out, b: inv.out }
"#,
        );
        // The "inv.out" reference in result should be resolved to "inv/gate"
        let result = expanded
            .def
            .components
            .iter()
            .find(|c| c.name == "result")
            .unwrap();
        let inputs = result.inputs.as_ref().unwrap();
        assert_eq!(inputs["a"], "inv/gate");
        assert_eq!(inputs["b"], "inv/gate");
    }

    #[test]
    fn expand_three_levels_deep() {
        let expanded = expand_ok(
            r#"
name: test
steps: 0
abstractions:
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
  - name: and
    inputs: [a, b]
    outputs: { out: inv.out }
    components:
      - type: nand
        name: nab
        inputs: { a: a, b: b }
      - type: not
        name: inv
        inputs: { x: nab }
  - name: or
    inputs: [a, b]
    outputs: { out: nand_notab }
    components:
      - type: not
        name: na
        inputs: { x: a }
      - type: not
        name: nb
        inputs: { x: b }
      - type: nand
        name: nand_notab
        inputs: { a: na.out, b: nb.out }
components:
  - type: dff
    name: x
    init: true
  - type: dff
    name: y
    init: false
  - type: or
    name: my_or
    inputs: { a: x, b: y }
"#,
        );
        // or contains: not na, not nb, nand nand_notab
        // Each not expands to one nand
        // So total: dff x, dff y, nand my_or/na/gate, nand my_or/nb/gate, nand my_or/nand_notab
        assert_eq!(expanded.def.components.len(), 5);
        assert_eq!(expanded.aliases["my_or.out"], "my_or/nand_notab");
    }

    #[test]
    fn abstraction_with_no_inputs() {
        let expanded = expand_ok(
            r#"
name: test
steps: 0
abstractions:
  - name: const_one
    inputs: []
    outputs: { out: val }
    components:
      - type: dff
        name: val
        init: true
components:
  - type: const_one
    name: one
"#,
        );
        assert_eq!(expanded.def.components.len(), 1);
        assert_eq!(expanded.def.components[0].name, "one/val");
        assert_eq!(expanded.aliases["one.out"], "one/val");
    }
}
