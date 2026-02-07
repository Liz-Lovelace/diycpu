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
pub fn expand(def: &CircuitDef) -> Result<ExpandedCircuit, ExpandError> {
    if def.abstractions.is_empty() {
        // Still validate that all component types are primitives.
        for comp in &def.components {
            if !is_primitive(&comp.component_type) {
                return Err(ExpandError::UnknownType {
                    component: comp.name.clone(),
                    type_name: comp.component_type.clone(),
                });
            }
        }
        return Ok(ExpandedCircuit {
            def: def.clone(),
            aliases: HashMap::new(),
        });
    }

    let abs_map: HashMap<&str, &AbstractionDef> = def
        .abstractions
        .iter()
        .map(|a| (a.name.as_str(), a))
        .collect();

    validate_abstractions(&def.abstractions, &abs_map)?;

    let mut flat = Vec::new();
    let mut aliases: HashMap<String, String> = HashMap::new();

    for comp in &def.components {
        if is_primitive(&comp.component_type) {
            flat.push(comp.clone());
        } else if let Some(abs) = abs_map.get(comp.component_type.as_str()) {
            let input_bindings = comp.inputs.as_ref().cloned().unwrap_or_default();

            validate_instance_inputs(&comp.name, abs, &input_bindings)?;

            let output_map =
                expand_instance(&comp.name, abs, &input_bindings, &abs_map, &mut flat)?;

            for (out_name, out_flat) in output_map {
                aliases.insert(format!("{}.{}", comp.name, out_name), out_flat);
            }
        } else {
            return Err(ExpandError::UnknownType {
                component: comp.name.clone(),
                type_name: comp.component_type.clone(),
            });
        }
    }

    // Resolve dot-notation references in all flat components.
    for comp in &mut flat {
        if let Some(inputs) = &mut comp.inputs {
            for val in inputs.values_mut() {
                if let Some(resolved) = aliases.get(val.as_str()) {
                    *val = resolved.clone();
                }
            }
        }
    }

    Ok(ExpandedCircuit {
        def: CircuitDef {
            name: def.name.clone(),
            steps: def.steps,
            abstractions: vec![],
            components: flat,
        },
        aliases,
    })
}

fn validate_abstractions(
    abstractions: &[AbstractionDef],
    abs_map: &HashMap<&str, &AbstractionDef>,
) -> Result<(), ExpandError> {
    // Check for duplicate names.
    let mut seen = HashSet::new();
    for abs in abstractions {
        if !seen.insert(&abs.name) {
            return Err(ExpandError::DuplicateAbstraction(abs.name.clone()));
        }
    }

    // Check for CLK inside abstractions.
    for abs in abstractions {
        check_no_clk(abs, abs_map)?;
    }

    // Check for self-references and circular dependencies.
    check_abstraction_cycles(abstractions, abs_map)?;

    Ok(())
}

/// Recursively check that no CLK component exists inside an abstraction
/// (including inside nested abstraction components).
fn check_no_clk(
    abs: &AbstractionDef,
    _abs_map: &HashMap<&str, &AbstractionDef>,
) -> Result<(), ExpandError> {
    for comp in &abs.components {
        if comp.component_type == "clk" {
            return Err(ExpandError::ClkInAbstraction(abs.name.clone()));
        }
    }
    Ok(())
}

fn check_abstraction_cycles(
    abstractions: &[AbstractionDef],
    abs_map: &HashMap<&str, &AbstractionDef>,
) -> Result<(), ExpandError> {
    let abs_names: HashSet<&str> = abs_map.keys().copied().collect();

    // Build dependency graph: abstraction -> set of abstractions it uses.
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for abs in abstractions {
        in_degree.entry(abs.name.as_str()).or_insert(0);
        dependents.entry(abs.name.as_str()).or_default();

        for comp in &abs.components {
            if abs_names.contains(comp.component_type.as_str()) {
                if comp.component_type == abs.name {
                    return Err(ExpandError::SelfReferencingAbstraction(abs.name.clone()));
                }
                *in_degree.entry(abs.name.as_str()).or_insert(0) += 1;
                dependents
                    .entry(comp.component_type.as_str())
                    .or_default()
                    .push(abs.name.as_str());
            }
        }
    }

    // Kahn's algorithm.
    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&name, _)| name)
        .collect();

    let mut sorted = 0;
    while let Some(name) = queue.pop_front() {
        sorted += 1;
        if let Some(deps) = dependents.get(name) {
            for &dep in deps {
                let deg = in_degree.get_mut(dep).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(dep);
                }
            }
        }
    }

    if sorted != abstractions.len() {
        return Err(ExpandError::CircularAbstractions);
    }

    Ok(())
}

fn validate_instance_inputs(
    instance_name: &str,
    abs: &AbstractionDef,
    input_bindings: &HashMap<String, String>,
) -> Result<(), ExpandError> {
    // Check all declared inputs are provided.
    for input in &abs.inputs {
        if !input_bindings.contains_key(input) {
            return Err(ExpandError::MissingAbstractionInput {
                instance: instance_name.to_string(),
                abstraction: abs.name.clone(),
                input: input.clone(),
            });
        }
    }

    // Check no extra inputs.
    let declared: HashSet<&str> = abs.inputs.iter().map(|s| s.as_str()).collect();
    for key in input_bindings.keys() {
        if !declared.contains(key.as_str()) {
            return Err(ExpandError::ExtraAbstractionInput {
                instance: instance_name.to_string(),
                abstraction: abs.name.clone(),
                input: key.clone(),
            });
        }
    }

    Ok(())
}

/// Expand one abstraction instance. Returns output_name -> flat_component_name mapping.
fn expand_instance(
    prefix: &str,
    abs: &AbstractionDef,
    input_bindings: &HashMap<String, String>,
    abs_map: &HashMap<&str, &AbstractionDef>,
    flat: &mut Vec<ComponentDef>,
) -> Result<HashMap<String, String>, ExpandError> {
    // Phase 1: Register all internal component flat names.
    let mut internal_map: HashMap<String, String> = HashMap::new();
    for comp in &abs.components {
        internal_map.insert(comp.name.clone(), format!("{prefix}/{}", comp.name));
    }

    // Phase 2: Expand sub-abstraction instances (in definition order)
    // to learn their output aliases.
    for comp in &abs.components {
        if !is_primitive(&comp.component_type) {
            let sub_abs = abs_map.get(comp.component_type.as_str()).ok_or_else(|| {
                ExpandError::UnknownType {
                    component: format!("{prefix}/{}", comp.name),
                    type_name: comp.component_type.clone(),
                }
            })?;

            let sub_input_bindings = resolve_input_map(
                &format!("{prefix}/{}", comp.name),
                comp.inputs.as_ref(),
                input_bindings,
                &internal_map,
            )?;

            validate_instance_inputs(
                &format!("{prefix}/{}", comp.name),
                sub_abs,
                &sub_input_bindings,
            )?;

            let sub_outputs = expand_instance(
                &format!("{prefix}/{}", comp.name),
                sub_abs,
                &sub_input_bindings,
                abs_map,
                flat,
            )?;

            // Register sub-output aliases: "comp_name.out_name" -> flat name.
            for (out_name, out_flat) in sub_outputs {
                internal_map.insert(format!("{}.{}", comp.name, out_name), out_flat);
            }
        }
    }

    // Phase 3: Add primitive components with resolved inputs.
    for comp in &abs.components {
        if is_primitive(&comp.component_type) {
            let flat_name = format!("{prefix}/{}", comp.name);
            let resolved_inputs = resolve_input_map(
                &flat_name,
                comp.inputs.as_ref(),
                input_bindings,
                &internal_map,
            )?;

            flat.push(ComponentDef {
                component_type: comp.component_type.clone(),
                name: flat_name,
                init: comp.init,
                inputs: if resolved_inputs.is_empty() {
                    None
                } else {
                    Some(resolved_inputs)
                },
                pos: None,
            });
        }
    }

    // Phase 4: Resolve output mapping.
    let mut output_map = HashMap::new();
    for (out_name, internal_ref) in &abs.outputs {
        let resolved = resolve_ref(
            &format!("{prefix} output \"{out_name}\""),
            internal_ref,
            input_bindings,
            &internal_map,
        )?;
        output_map.insert(out_name.clone(), resolved);
    }

    Ok(output_map)
}

fn resolve_input_map(
    context: &str,
    inputs: Option<&HashMap<String, String>>,
    input_bindings: &HashMap<String, String>,
    internal_map: &HashMap<String, String>,
) -> Result<HashMap<String, String>, ExpandError> {
    let Some(inputs) = inputs else {
        return Ok(HashMap::new());
    };
    inputs
        .iter()
        .map(|(k, v)| {
            let resolved = resolve_ref(context, v, input_bindings, internal_map)?;
            Ok((k.clone(), resolved))
        })
        .collect()
}

fn resolve_ref(
    context: &str,
    reference: &str,
    input_bindings: &HashMap<String, String>,
    internal_map: &HashMap<String, String>,
) -> Result<String, ExpandError> {
    // First: is it an abstraction input parameter?
    if let Some(resolved) = input_bindings.get(reference) {
        return Ok(resolved.clone());
    }
    // Second: is it an internal component or sub-output alias?
    if let Some(resolved) = internal_map.get(reference) {
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
        let ec = expand_ok(
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
        assert_eq!(ec.def.components.len(), 2); // dff + expanded nand
        let nand = &ec.def.components[1];
        assert_eq!(nand.name, "inv/gate");
        assert_eq!(nand.component_type, "nand");
        let inputs = nand.inputs.as_ref().unwrap();
        assert_eq!(inputs["a"], "a");
        assert_eq!(inputs["b"], "a");
        assert_eq!(ec.aliases["inv.out"], "inv/gate");
    }

    #[test]
    fn expand_nested_and() {
        let ec = expand_ok(
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
        assert_eq!(ec.def.components.len(), 4);

        let names: Vec<&str> = ec.def.components.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"my_and/nand_ab"));
        assert!(names.contains(&"my_and/inv/gate"));

        // Check alias resolution
        assert_eq!(ec.aliases["my_and.out"], "my_and/inv/gate");
    }

    #[test]
    fn expand_preserves_init() {
        let ec = expand_ok(
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
        let ff = ec
            .def
            .components
            .iter()
            .find(|c| c.name == "my_reg/ff")
            .unwrap();
        assert_eq!(ff.init, Some(true));
    }

    #[test]
    fn no_abstractions_passthrough() {
        let ec = expand_ok(
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
        assert_eq!(ec.def.components.len(), 2);
        assert!(ec.aliases.is_empty());
    }

    #[test]
    fn expand_with_dot_reference_in_top_level() {
        let ec = expand_ok(
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
        let result = ec
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
        let ec = expand_ok(
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
        assert_eq!(ec.def.components.len(), 5);
        assert_eq!(ec.aliases["my_or.out"], "my_or/nand_notab");
    }

    #[test]
    fn abstraction_with_no_inputs() {
        let ec = expand_ok(
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
        assert_eq!(ec.def.components.len(), 1);
        assert_eq!(ec.def.components[0].name, "one/val");
        assert_eq!(ec.aliases["one.out"], "one/val");
    }
}
