use std::collections::HashMap;

use crate::compiler::{CompiledCircuit, CompiledComponent};

/// Holds the live state of a simulation: current output values for every
/// component and previous clock-input values for DFF edge detection.
pub struct Simulator<'a> {
    circuit: &'a CompiledCircuit,
    /// Current output value for each component, indexed by component position.
    component_values: Vec<bool>,
    /// Previous clock-input value for each DFF (indexed by position in dff_indices).
    previous_dff_clock_values: Vec<bool>,
}

impl<'a> Simulator<'a> {
    pub fn new(circuit: &'a CompiledCircuit) -> Self {
        let component_count = circuit.components.len();
        let mut component_values = vec![false; component_count];

        initialise_dff_outputs(circuit, &mut component_values);

        // CLK starts at 0 (already false by default).

        // Run an initial evaluation so NAND outputs are consistent with inputs.
        evaluate_combinational_logic(circuit, &mut component_values);

        // Snapshot each DFF's clock input so we can detect edges on the first step.
        let previous_dff_clock_values = snapshot_dff_clock_inputs(circuit, &component_values);

        Simulator {
            circuit,
            component_values,
            previous_dff_clock_values,
        }
    }

    /// Advance one simulation step: toggle CLK, evaluate, latch DFFs on rising
    /// edges, then re-evaluate so displayed values are consistent.
    pub fn step(&mut self) {
        self.toggle_clock();

        // Pass 1: evaluate combinational logic with old DFF values.
        evaluate_combinational_logic(self.circuit, &mut self.component_values);

        // Detect rising edges and latch DFF data inputs.
        let any_dff_latched =
            self.latch_dffs_on_rising_edges();

        // Pass 2: re-evaluate with new DFF values so the display is consistent.
        if any_dff_latched {
            evaluate_combinational_logic(self.circuit, &mut self.component_values);
        }

        // Record clock inputs for next step's edge detection.
        self.previous_dff_clock_values =
            snapshot_dff_clock_inputs(self.circuit, &self.component_values);
    }

    /// Return the current output value of every named component.
    pub fn current_state(&self) -> HashMap<String, bool> {
        self.circuit
            .component_names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.clone(), self.component_values[index]))
            .collect()
    }

    // ── Private helpers ──────────────────────────────────────────────────

    fn toggle_clock(&mut self) {
        if let Some(clock_index) = self.circuit.clock_index {
            self.component_values[clock_index] = !self.component_values[clock_index];
        }
    }

    /// Check each DFF for a 0→1 (rising) edge on its clock input. If found,
    /// latch the current data-input value. Returns true if any DFF latched.
    fn latch_dffs_on_rising_edges(&mut self) -> bool {
        let mut any_latched = false;

        for (dff_position, &component_index) in self.circuit.dff_indices.iter().enumerate() {
            let current_clock = read_dff_clock_input(
                self.circuit,
                &self.component_values,
                component_index,
            );
            let previous_clock = self.previous_dff_clock_values[dff_position];

            let is_rising_edge = !previous_clock && current_clock;
            if is_rising_edge {
                let data_value = read_dff_data_input(
                    self.circuit,
                    &self.component_values,
                    component_index,
                );
                self.component_values[component_index] = data_value;
                any_latched = true;
            }
        }

        any_latched
    }
}

// ── Free functions ───────────────────────────────────────────────────────────

/// Set each DFF's output to its declared init value (defaults to false).
fn initialise_dff_outputs(circuit: &CompiledCircuit, component_values: &mut [bool]) {
    for &dff_index in &circuit.dff_indices {
        if let CompiledComponent::Dff { init, .. } = &circuit.components[dff_index] {
            component_values[dff_index] = *init;
        }
    }
}

/// Record the current clock-input value for every DFF, so the next step
/// can detect rising edges.
fn snapshot_dff_clock_inputs(
    circuit: &CompiledCircuit,
    component_values: &[bool],
) -> Vec<bool> {
    circuit
        .dff_indices
        .iter()
        .map(|&dff_index| read_dff_clock_input(circuit, component_values, dff_index))
        .collect()
}

/// Walk the evaluation order and compute every NAND gate output.
/// CLK and DFF values must already be set in `component_values`.
fn evaluate_combinational_logic(circuit: &CompiledCircuit, component_values: &mut [bool]) {
    for &component_index in &circuit.evaluation_order {
        if let CompiledComponent::Nand { input_a, input_b } =
            &circuit.components[component_index]
        {
            component_values[component_index] =
                !(component_values[*input_a] && component_values[*input_b]);
        }
    }
}

/// Read the current clock-input value for a DFF. Returns false if unconnected.
fn read_dff_clock_input(
    circuit: &CompiledCircuit,
    component_values: &[bool],
    dff_index: usize,
) -> bool {
    match &circuit.components[dff_index] {
        CompiledComponent::Dff {
            input_clk: Some(clock_source_index),
            ..
        } => component_values[*clock_source_index],
        _ => false,
    }
}

/// Read the current data-input value for a DFF. Returns false if unconnected.
fn read_dff_data_input(
    circuit: &CompiledCircuit,
    component_values: &[bool],
    dff_index: usize,
) -> bool {
    match &circuit.components[dff_index] {
        CompiledComponent::Dff {
            input_d: Some(data_source_index),
            ..
        } => component_values[*data_source_index],
        _ => false,
    }
}

/// Run the full simulation for all steps, returning a Vec of state snapshots.
/// Index 0 is the initial state before any clock toggles.
pub fn simulate(circuit: &CompiledCircuit) -> Vec<HashMap<String, bool>> {
    let mut simulator = Simulator::new(circuit);
    let mut results = Vec::with_capacity(circuit.steps + 1);

    // Step 0: initial state before any toggles.
    results.push(simulator.current_state());

    // Steps 1..=N: one clock toggle per step.
    for _ in 0..circuit.steps {
        simulator.step();
        results.push(simulator.current_state());
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::expander::expand;
    use crate::types::CircuitDef;

    /// Parse YAML, expand abstractions, compile, simulate, and inject output aliases.
    fn run(yaml: &str) -> Vec<HashMap<String, bool>> {
        let circuit_def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        let expanded = expand(&circuit_def).unwrap();
        let compiled_circuit = compile(&expanded.def).unwrap();
        let mut results = simulate(&compiled_circuit);

        // Inject output aliases so abstraction outputs are accessible by dot-notation.
        for step_state in &mut results {
            for (alias_name, flat_target) in &expanded.aliases {
                if let Some(&value) = step_state.get(flat_target) {
                    step_state.insert(alias_name.clone(), value);
                }
            }
        }
        results
    }

    #[test]
    fn constant_dff() {
        let results = run(
            r#"
name: constant
steps: 4
components:
  - type: dff
    name: high
    init: true
  - type: dff
    name: low
    init: false
"#,
        );
        for step_state in &results {
            assert_eq!(step_state["high"], true);
            assert_eq!(step_state["low"], false);
        }
    }

    #[test]
    fn not_gate() {
        let results = run(
            r#"
name: not
steps: 2
components:
  - type: dff
    name: input
    init: true
  - type: nand
    name: output
    inputs: { a: input, b: input }
"#,
        );
        // NAND(1,1) = 0
        for step_state in &results {
            assert_eq!(step_state["input"], true);
            assert_eq!(step_state["output"], false);
        }
    }

    #[test]
    fn toggle_flip_flop() {
        let results = run(
            r#"
name: toggle
steps: 8
components:
  - type: clk
    name: clk
  - type: dff
    name: state
    init: false
    inputs:
      d: not_state
      clk: clk
  - type: nand
    name: not_state
    inputs: { a: state, b: state }
"#,
        );

        // Step 0: CLK=0, state=0, not_state=1
        assert_eq!(results[0]["clk"], false);
        assert_eq!(results[0]["state"], false);
        assert_eq!(results[0]["not_state"], true);

        // Step 1: CLK=1 (rising edge), state latches not_state=1
        assert_eq!(results[1]["clk"], true);
        assert_eq!(results[1]["state"], true);
        assert_eq!(results[1]["not_state"], false);

        // Step 2: CLK=0 (falling edge), no latch
        assert_eq!(results[2]["clk"], false);
        assert_eq!(results[2]["state"], true);
        assert_eq!(results[2]["not_state"], false);

        // Step 3: CLK=1 (rising edge), state latches not_state=0
        assert_eq!(results[3]["clk"], true);
        assert_eq!(results[3]["state"], false);
        assert_eq!(results[3]["not_state"], true);

        // Pattern repeats: state toggles on each rising edge.
        assert_eq!(results[4]["state"], false);
        assert_eq!(results[5]["state"], true);
        assert_eq!(results[6]["state"], true);
        assert_eq!(results[7]["state"], false);
    }

    #[test]
    fn and_gate() {
        // AND(a,b) = NOT(NAND(a,b))
        let results = run(
            r#"
name: and
steps: 0
components:
  - type: dff
    name: a
    init: true
  - type: dff
    name: b
    init: true
  - type: nand
    name: nand_ab
    inputs: { a: a, b: b }
  - type: nand
    name: and_ab
    inputs: { a: nand_ab, b: nand_ab }
"#,
        );
        assert_eq!(results[0]["and_ab"], true);
    }

    #[test]
    fn and_gate_false_cases() {
        let results = run(
            r#"
name: and_10
steps: 0
components:
  - type: dff
    name: a
    init: true
  - type: dff
    name: b
    init: false
  - type: nand
    name: nand_ab
    inputs: { a: a, b: b }
  - type: nand
    name: and_ab
    inputs: { a: nand_ab, b: nand_ab }
"#,
        );
        assert_eq!(results[0]["and_ab"], false);
    }

    #[test]
    fn or_gate() {
        // OR(a,b) = NAND(NOT(a), NOT(b))
        let results = run(
            r#"
name: or
steps: 0
components:
  - type: dff
    name: a
    init: false
  - type: dff
    name: b
    init: true
  - type: nand
    name: not_a
    inputs: { a: a, b: a }
  - type: nand
    name: not_b
    inputs: { a: b, b: b }
  - type: nand
    name: or_ab
    inputs: { a: not_a, b: not_b }
"#,
        );
        assert_eq!(results[0]["or_ab"], true);
    }

    #[test]
    fn or_gate_both_false() {
        let results = run(
            r#"
name: or_00
steps: 0
components:
  - type: dff
    name: a
    init: false
  - type: dff
    name: b
    init: false
  - type: nand
    name: not_a
    inputs: { a: a, b: a }
  - type: nand
    name: not_b
    inputs: { a: b, b: b }
  - type: nand
    name: or_ab
    inputs: { a: not_a, b: not_b }
"#,
        );
        assert_eq!(results[0]["or_ab"], false);
    }

    #[test]
    fn no_clk_circuit() {
        let results = run(
            r#"
name: no_clk
steps: 3
components:
  - type: dff
    name: one
    init: true
  - type: dff
    name: zero
    init: false
  - type: nand
    name: result
    inputs: { a: one, b: zero }
"#,
        );
        // NAND(1,0) = 1, stable across all steps.
        for step_state in &results {
            assert_eq!(step_state["result"], true);
        }
    }

    #[test]
    fn dff_d_without_clk_never_latches() {
        let results = run(
            r#"
name: no_latch
steps: 4
components:
  - type: dff
    name: source
    init: true
  - type: dff
    name: target
    init: false
    inputs:
      d: source
"#,
        );
        // target has d=source but no clk, so it never latches.
        for step_state in &results {
            assert_eq!(step_state["target"], false);
        }
    }

    #[test]
    fn rising_edge_only() {
        let results = run(
            r#"
name: edge_test
steps: 6
components:
  - type: clk
    name: clk
  - type: dff
    name: one
    init: true
  - type: dff
    name: reg
    init: false
    inputs:
      d: one
      clk: clk
"#,
        );
        assert_eq!(results[0]["reg"], false);
        assert_eq!(results[1]["reg"], true); // rising edge latches
        assert_eq!(results[2]["reg"], true); // falling edge, no change
        assert_eq!(results[3]["reg"], true);
        assert_eq!(results[4]["reg"], true);
        assert_eq!(results[5]["reg"], true);
    }

    #[test]
    fn multiple_dffs_independent() {
        let results = run(
            r#"
name: multi_dff
steps: 4
components:
  - type: clk
    name: clk
  - type: dff
    name: a
    init: true
  - type: dff
    name: b
    init: false
  - type: nand
    name: not_a
    inputs: { a: a, b: a }
  - type: nand
    name: not_b
    inputs: { a: b, b: b }
  - type: dff
    name: reg_a
    init: false
    inputs: { d: not_a, clk: clk }
  - type: dff
    name: reg_b
    init: false
    inputs: { d: not_b, clk: clk }
"#,
        );
        assert_eq!(results[0]["reg_a"], false);
        assert_eq!(results[0]["reg_b"], false);
        // Rising edge: reg_a latches NOT(a)=0, reg_b latches NOT(b)=1.
        assert_eq!(results[1]["reg_a"], false);
        assert_eq!(results[1]["reg_b"], true);
    }

    #[test]
    fn dff_self_reference_holds_value() {
        let results = run(
            r#"
name: self_hold
steps: 4
components:
  - type: clk
    name: clk
  - type: dff
    name: reg
    init: true
    inputs:
      d: reg
      clk: clk
"#,
        );
        for step_state in &results {
            assert_eq!(step_state["reg"], true);
        }
    }

    #[test]
    fn xor_gate() {
        for (a_init, b_init, expected) in
            [(false, false, false), (false, true, true), (true, false, true), (true, true, false)]
        {
            let yaml = format!(
                r#"
name: xor
steps: 0
components:
  - type: dff
    name: a
    init: {a_init}
  - type: dff
    name: b
    init: {b_init}
  - type: nand
    name: nand_ab
    inputs: {{ a: a, b: b }}
  - type: nand
    name: nand_a_nab
    inputs: {{ a: a, b: nand_ab }}
  - type: nand
    name: nand_b_nab
    inputs: {{ a: b, b: nand_ab }}
  - type: nand
    name: xor
    inputs: {{ a: nand_a_nab, b: nand_b_nab }}
"#,
            );
            let results = run(&yaml);
            assert_eq!(
                results[0]["xor"], expected,
                "XOR({a_init}, {b_init}) should be {expected}"
            );
        }
    }

    #[test]
    fn two_bit_counter() {
        let results = run(
            r#"
name: counter
steps: 16
components:
  - type: clk
    name: clk
  - type: dff
    name: bit0
    init: false
    inputs: { d: not_bit0, clk: clk }
  - type: dff
    name: bit1
    init: false
    inputs: { d: xor_01, clk: clk }
  - type: nand
    name: not_bit0
    inputs: { a: bit0, b: bit0 }
  - type: nand
    name: nand_01
    inputs: { a: bit0, b: bit1 }
  - type: nand
    name: nand_0_n01
    inputs: { a: bit0, b: nand_01 }
  - type: nand
    name: nand_1_n01
    inputs: { a: bit1, b: nand_01 }
  - type: nand
    name: xor_01
    inputs: { a: nand_0_n01, b: nand_1_n01 }
"#,
        );

        let expected_bit0 = [
            false, true, true, false, false, true, true, false, false, true, true, false, false,
            true, true, false, false,
        ];
        let expected_bit1 = [
            false, false, false, true, true, true, true, false, false, false, false, true, true,
            true, true, false, false,
        ];

        for (step_number, step_state) in results.iter().enumerate() {
            assert_eq!(
                step_state["bit0"], expected_bit0[step_number],
                "bit0 mismatch at step {step_number}"
            );
            assert_eq!(
                step_state["bit1"], expected_bit1[step_number],
                "bit1 mismatch at step {step_number}"
            );
        }
    }

    #[test]
    fn step_count_matches() {
        let results = run(
            r#"
name: count_test
steps: 5
components:
  - type: dff
    name: x
"#,
        );
        // steps=5 means 5 toggles + initial state = 6 rows.
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn zero_steps() {
        let results = run(
            r#"
name: zero
steps: 0
components:
  - type: dff
    name: x
    init: true
"#,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["x"], true);
    }

    #[test]
    fn empty_circuit() {
        let results = run(
            r#"
name: empty
steps: 3
components: []
"#,
        );
        assert_eq!(results.len(), 4);
        for step_state in &results {
            assert!(step_state.is_empty());
        }
    }

    // ── Abstraction end-to-end tests ─────────────────────────────────────

    fn abs_prelude() -> &'static str {
        r#"
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
    outputs: { out: result }
    components:
      - type: not
        name: na
        inputs: { x: a }
      - type: not
        name: nb
        inputs: { x: b }
      - type: nand
        name: result
        inputs: { a: na.out, b: nb.out }
  - name: xor
    inputs: [a, b]
    outputs: { out: result }
    components:
      - type: nand
        name: nab
        inputs: { a: a, b: b }
      - type: nand
        name: na_nab
        inputs: { a: a, b: nab }
      - type: nand
        name: nb_nab
        inputs: { a: b, b: nab }
      - type: nand
        name: result
        inputs: { a: na_nab, b: nb_nab }
"#
    }

    #[test]
    fn abs_not_gate() {
        for (input_value, expected_output) in [(false, true), (true, false)] {
            let yaml = format!(
                r#"
name: abs_not
steps: 0
{prelude}
components:
  - type: dff
    name: x
    init: {input_value}
  - type: not
    name: inv
    inputs: {{ x: x }}
"#,
                prelude = abs_prelude(),
            );
            let results = run(&yaml);
            assert_eq!(
                results[0]["inv.out"], expected_output,
                "NOT({input_value}) should be {expected_output}"
            );
        }
    }

    #[test]
    fn abs_and_gate() {
        for (a, b, expected) in [
            (false, false, false),
            (false, true, false),
            (true, false, false),
            (true, true, true),
        ] {
            let yaml = format!(
                r#"
name: abs_and
steps: 0
{prelude}
components:
  - type: dff
    name: a
    init: {a}
  - type: dff
    name: b
    init: {b}
  - type: and
    name: g
    inputs: {{ a: a, b: b }}
"#,
                prelude = abs_prelude(),
            );
            let results = run(&yaml);
            assert_eq!(
                results[0]["g.out"], expected,
                "AND({a}, {b}) should be {expected}"
            );
        }
    }

    #[test]
    fn abs_or_gate() {
        for (a, b, expected) in [
            (false, false, false),
            (false, true, true),
            (true, false, true),
            (true, true, true),
        ] {
            let yaml = format!(
                r#"
name: abs_or
steps: 0
{prelude}
components:
  - type: dff
    name: a
    init: {a}
  - type: dff
    name: b
    init: {b}
  - type: or
    name: g
    inputs: {{ a: a, b: b }}
"#,
                prelude = abs_prelude(),
            );
            let results = run(&yaml);
            assert_eq!(
                results[0]["g.out"], expected,
                "OR({a}, {b}) should be {expected}"
            );
        }
    }

    #[test]
    fn abs_xor_gate() {
        for (a, b, expected) in [
            (false, false, false),
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            let yaml = format!(
                r#"
name: abs_xor
steps: 0
{prelude}
components:
  - type: dff
    name: a
    init: {a}
  - type: dff
    name: b
    init: {b}
  - type: xor
    name: g
    inputs: {{ a: a, b: b }}
"#,
                prelude = abs_prelude(),
            );
            let results = run(&yaml);
            assert_eq!(
                results[0]["g.out"], expected,
                "XOR({a}, {b}) should be {expected}"
            );
        }
    }

    #[test]
    fn abs_toggle_with_not() {
        let yaml = format!(
            r#"
name: abs_toggle
steps: 8
{prelude}
components:
  - type: clk
    name: clk
  - type: dff
    name: state
    init: false
    inputs:
      d: inv.out
      clk: clk
  - type: not
    name: inv
    inputs: {{ x: state }}
"#,
            prelude = abs_prelude(),
        );
        let results = run(&yaml);
        assert_eq!(results[0]["state"], false);
        assert_eq!(results[1]["state"], true);
        assert_eq!(results[2]["state"], true);
        assert_eq!(results[3]["state"], false);
        assert_eq!(results[4]["state"], false);
        assert_eq!(results[5]["state"], true);
    }

    #[test]
    fn abs_counter_with_xor_and_not() {
        let yaml = format!(
            r#"
name: abs_counter
steps: 16
{prelude}
components:
  - type: clk
    name: clk
  - type: dff
    name: bit0
    init: false
    inputs: {{ d: n0.out, clk: clk }}
  - type: dff
    name: bit1
    init: false
    inputs: {{ d: x01.out, clk: clk }}
  - type: not
    name: n0
    inputs: {{ x: bit0 }}
  - type: xor
    name: x01
    inputs: {{ a: bit0, b: bit1 }}
"#,
            prelude = abs_prelude(),
        );
        let results = run(&yaml);

        let expected_bit0 = [
            false, true, true, false, false, true, true, false, false, true, true, false, false,
            true, true, false, false,
        ];
        let expected_bit1 = [
            false, false, false, true, true, true, true, false, false, false, false, true, true,
            true, true, false, false,
        ];

        for (step_number, step_state) in results.iter().enumerate() {
            assert_eq!(
                step_state["bit0"], expected_bit0[step_number],
                "bit0 mismatch at step {step_number}"
            );
            assert_eq!(
                step_state["bit1"], expected_bit1[step_number],
                "bit1 mismatch at step {step_number}"
            );
        }
    }

    #[test]
    fn abs_dff_inside_abstraction() {
        let yaml = r#"
name: dff_in_abs
steps: 4
abstractions:
  - name: register
    inputs: [d, clk]
    outputs: { q: ff }
    components:
      - type: dff
        name: ff
        inputs: { d: d, clk: clk }
components:
  - type: clk
    name: clk
  - type: dff
    name: one
    init: true
  - type: register
    name: my_reg
    inputs: { d: one, clk: clk }
"#;
        let results = run(yaml);
        assert_eq!(results[0]["my_reg.q"], false);
        assert_eq!(results[1]["my_reg.q"], true); // rising edge latches d=one=1
        assert_eq!(results[2]["my_reg.q"], true);
        assert_eq!(results[3]["my_reg.q"], true);
    }

    #[test]
    fn abs_multiple_outputs() {
        let yaml = r#"
name: multi_out
steps: 0
abstractions:
  - name: split
    inputs: [x]
    outputs:
      normal: buf.out
      inverted: inv.out
    components:
      - type: not
        name: inv
        inputs: { x: x }
      - type: not
        name: buf
        inputs: { x: inv.out }
  - name: not
    inputs: [x]
    outputs: { out: gate }
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
components:
  - type: dff
    name: val
    init: true
  - type: split
    name: s
    inputs: { x: val }
"#;
        let results = run(yaml);
        assert_eq!(results[0]["s.normal"], true);
        assert_eq!(results[0]["s.inverted"], false);
    }

    #[test]
    fn abs_full_adder() {
        let build_full_adder_yaml = |a: bool, b: bool, cin: bool| {
            format!(
                r#"
name: full_adder
steps: 0
abstractions:
  - name: not
    inputs: [x]
    outputs: {{ out: gate }}
    components:
      - type: nand
        name: gate
        inputs: {{ a: x, b: x }}
  - name: and
    inputs: [a, b]
    outputs: {{ out: inv.out }}
    components:
      - type: nand
        name: nab
        inputs: {{ a: a, b: b }}
      - type: not
        name: inv
        inputs: {{ x: nab }}
  - name: or
    inputs: [a, b]
    outputs: {{ out: result }}
    components:
      - type: not
        name: na
        inputs: {{ x: a }}
      - type: not
        name: nb
        inputs: {{ x: b }}
      - type: nand
        name: result
        inputs: {{ a: na.out, b: nb.out }}
  - name: xor
    inputs: [a, b]
    outputs: {{ out: result }}
    components:
      - type: nand
        name: nab
        inputs: {{ a: a, b: b }}
      - type: nand
        name: na_nab
        inputs: {{ a: a, b: nab }}
      - type: nand
        name: nb_nab
        inputs: {{ a: b, b: nab }}
      - type: nand
        name: result
        inputs: {{ a: na_nab, b: nb_nab }}
  - name: full_add
    inputs: [a, b, cin]
    outputs:
      sum: xor2.out
      cout: cout_or.out
    components:
      - type: xor
        name: xor1
        inputs: {{ a: a, b: b }}
      - type: xor
        name: xor2
        inputs: {{ a: xor1.out, b: cin }}
      - type: and
        name: and1
        inputs: {{ a: xor1.out, b: cin }}
      - type: and
        name: and2
        inputs: {{ a: a, b: b }}
      - type: or
        name: cout_or
        inputs: {{ a: and1.out, b: and2.out }}
components:
  - type: dff
    name: a
    init: {a}
  - type: dff
    name: b
    init: {b}
  - type: dff
    name: cin
    init: {cin}
  - type: full_add
    name: fa
    inputs: {{ a: a, b: b, cin: cin }}
"#,
            )
        };

        for (a, b, cin, expected_sum, expected_cout) in [
            (false, false, false, false, false),
            (false, false, true, true, false),
            (false, true, false, true, false),
            (false, true, true, false, true),
            (true, false, false, true, false),
            (true, false, true, false, true),
            (true, true, false, false, true),
            (true, true, true, true, true),
        ] {
            let results = run(&build_full_adder_yaml(a, b, cin));
            assert_eq!(
                results[0]["fa.sum"], expected_sum,
                "full_add({a},{b},{cin}).sum should be {expected_sum}"
            );
            assert_eq!(
                results[0]["fa.cout"], expected_cout,
                "full_add({a},{b},{cin}).cout should be {expected_cout}"
            );
        }
    }
}
