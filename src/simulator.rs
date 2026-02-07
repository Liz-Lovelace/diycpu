use std::collections::HashMap;

use crate::compiler::{CompiledCircuit, CompiledComponent};

pub struct Simulator<'a> {
    circuit: &'a CompiledCircuit,
    /// Current output value for each component.
    values: Vec<bool>,
    /// Previous clock-input value for each DFF (indexed by position in dff_indices).
    prev_clk: Vec<bool>,
}

impl<'a> Simulator<'a> {
    pub fn new(circuit: &'a CompiledCircuit) -> Self {
        let n = circuit.components.len();
        let mut values = vec![false; n];

        // Initialize DFF outputs to their init values.
        for &idx in &circuit.dff_indices {
            if let CompiledComponent::Dff { init, .. } = &circuit.components[idx] {
                values[idx] = *init;
            }
        }
        // CLK starts at 0 (already false).

        // Do an initial evaluation to establish NAND values.
        evaluate(circuit, &mut values);

        // Record initial clock-input values for each DFF.
        let prev_clk = circuit
            .dff_indices
            .iter()
            .map(|&idx| dff_clk_input(circuit, &values, idx))
            .collect();

        Simulator {
            circuit,
            values,
            prev_clk,
        }
    }

    /// Advance one simulation step: toggle CLK, evaluate, latch DFFs, re-evaluate.
    pub fn step(&mut self) {
        // Toggle CLK.
        if let Some(clk_idx) = self.circuit.clk_idx {
            self.values[clk_idx] = !self.values[clk_idx];
        }

        // Pass 1: evaluate combinational logic with old DFF values.
        evaluate(self.circuit, &mut self.values);

        // Edge detection and latching.
        let mut any_latched = false;
        for (dff_pos, &idx) in self.circuit.dff_indices.iter().enumerate() {
            let new_clk = dff_clk_input(self.circuit, &self.values, idx);
            let old_clk = self.prev_clk[dff_pos];

            if !old_clk && new_clk {
                // Rising edge: latch the current data input.
                let d_val = dff_d_input(self.circuit, &self.values, idx);
                self.values[idx] = d_val;
                any_latched = true;
            }
        }

        // Pass 2: re-evaluate with new DFF values (for consistent display).
        if any_latched {
            evaluate(self.circuit, &mut self.values);
        }

        // Record clock inputs for next step's edge detection.
        for (dff_pos, &idx) in self.circuit.dff_indices.iter().enumerate() {
            self.prev_clk[dff_pos] = dff_clk_input(self.circuit, &self.values, idx);
        }
    }

    /// Get the current state of all named components.
    pub fn current_state(&self) -> HashMap<String, bool> {
        self.circuit
            .names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), self.values[i]))
            .collect()
    }
}

/// Evaluate all components in topological order. CLK and DFF values are
/// already set in `values`; only NAND gates are computed.
fn evaluate(circuit: &CompiledCircuit, values: &mut [bool]) {
    for &idx in &circuit.eval_order {
        if let CompiledComponent::Nand { input_a, input_b } = &circuit.components[idx] {
            values[idx] = !(values[*input_a] && values[*input_b]);
        }
    }
}

/// Get the current clock-input value for a DFF. Returns false if unconnected.
fn dff_clk_input(circuit: &CompiledCircuit, values: &[bool], idx: usize) -> bool {
    match &circuit.components[idx] {
        CompiledComponent::Dff {
            input_clk: Some(clk_idx),
            ..
        } => values[*clk_idx],
        _ => false,
    }
}

/// Get the current data-input value for a DFF. Returns false if unconnected.
fn dff_d_input(circuit: &CompiledCircuit, values: &[bool], idx: usize) -> bool {
    match &circuit.components[idx] {
        CompiledComponent::Dff {
            input_d: Some(d_idx),
            ..
        } => values[*d_idx],
        _ => false,
    }
}

/// Run the full simulation, returning state at each step (step 0 = initial state).
pub fn simulate(circuit: &CompiledCircuit) -> Vec<HashMap<String, bool>> {
    let mut sim = Simulator::new(circuit);
    let mut results = Vec::with_capacity(circuit.steps + 1);

    // Step 0: initial state.
    results.push(sim.current_state());

    // Steps 1..=N.
    for _ in 0..circuit.steps {
        sim.step();
        results.push(sim.current_state());
    }

    results
}

/// Format simulation results as a text table.
pub fn format_table(circuit: &CompiledCircuit, results: &[HashMap<String, bool>]) -> String {
    let names = &circuit.names;
    let step_width = results.len().to_string().len().max(4);
    let col_widths: Vec<usize> = names.iter().map(|n| n.len().max(1)).collect();

    let mut out = String::new();

    // Header.
    out.push_str(&format!("{:>width$}", "Step", width = step_width));
    for (i, name) in names.iter().enumerate() {
        out.push_str(&format!(" | {:>width$}", name, width = col_widths[i]));
    }
    out.push('\n');

    // Separator.
    out.push_str(&"-".repeat(step_width));
    for w in &col_widths {
        out.push_str(&format!("-+-{}", "-".repeat(*w)));
    }
    out.push('\n');

    // Rows.
    for (step, state) in results.iter().enumerate() {
        out.push_str(&format!("{:>width$}", step, width = step_width));
        for (i, name) in names.iter().enumerate() {
            let val = if state[name] { 1 } else { 0 };
            out.push_str(&format!(" | {:>width$}", val, width = col_widths[i]));
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::expander::expand;
    use crate::types::CircuitDef;

    fn run(yaml: &str) -> Vec<HashMap<String, bool>> {
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        let expanded = expand(&def).unwrap();
        let circuit = compile(&expanded.def).unwrap();
        let mut results = simulate(&circuit);
        // Inject output aliases.
        for step in &mut results {
            for (alias, target) in &expanded.aliases {
                if let Some(&val) = step.get(target) {
                    step.insert(alias.clone(), val);
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
        for step in &results {
            assert_eq!(step["high"], true);
            assert_eq!(step["low"], false);
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
        for step in &results {
            assert_eq!(step["input"], true);
            assert_eq!(step["output"], false);
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

        // Step 1: CLK=1 (rising edge), state latches not_state=1 -> state=1, not_state=0
        assert_eq!(results[1]["clk"], true);
        assert_eq!(results[1]["state"], true);
        assert_eq!(results[1]["not_state"], false);

        // Step 2: CLK=0 (falling edge), no latch
        assert_eq!(results[2]["clk"], false);
        assert_eq!(results[2]["state"], true);
        assert_eq!(results[2]["not_state"], false);

        // Step 3: CLK=1 (rising edge), state latches not_state=0 -> state=0, not_state=1
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
        // AND(1,1) = 1
        assert_eq!(results[0]["and_ab"], true);
    }

    #[test]
    fn and_gate_false_cases() {
        // AND(1,0) = 0
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
        // OR(0,1) = 1
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
        // OR(0,0) = 0
        assert_eq!(results[0]["or_ab"], false);
    }

    #[test]
    fn no_clk_circuit() {
        // Circuit with no CLK: just constant DFFs feeding NANDs.
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
        // NAND(1,0) = 1 (stable across all steps)
        for step in &results {
            assert_eq!(step["result"], true);
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
        // target has d=source but no clk, so it never latches. Stays false.
        for step in &results {
            assert_eq!(step["target"], false);
        }
    }

    #[test]
    fn rising_edge_only() {
        // Verify DFF only latches on 0->1 transition, not 1->0.
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
        // Step 0: clk=0, reg=0
        assert_eq!(results[0]["reg"], false);
        // Step 1: clk=1 (rising edge), reg latches d=one=1 -> reg=1
        assert_eq!(results[1]["reg"], true);
        // Step 2: clk=0 (falling edge), no latch, reg stays 1
        assert_eq!(results[2]["reg"], true);
        // Step 3: clk=1 (rising edge), reg latches d=1 -> still 1
        assert_eq!(results[3]["reg"], true);
        // All subsequent: stays 1
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
        // Step 0: not_a=0, not_b=1, reg_a=0, reg_b=0
        assert_eq!(results[0]["reg_a"], false);
        assert_eq!(results[0]["reg_b"], false);
        // Step 1: rising edge. reg_a latches not_a=0, reg_b latches not_b=1
        assert_eq!(results[1]["reg_a"], false);
        assert_eq!(results[1]["reg_b"], true);
    }

    #[test]
    fn dff_self_reference_holds_value() {
        // DFF whose data input is its own output: latches itself, never changes.
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
        for step in &results {
            assert_eq!(step["reg"], true);
        }
    }

    #[test]
    fn xor_gate() {
        // XOR(a,b) = NAND(NAND(a, NAND(a,b)), NAND(b, NAND(a,b)))
        // Test all four input combinations.
        for (a_init, b_init, expected) in [(false, false, false), (false, true, true), (true, false, true), (true, true, false)] {
            let yaml = format!(
                r#"
name: xor
steps: 0
components:
  - type: dff
    name: a
    init: {a}
  - type: dff
    name: b
    init: {b}
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
                a = a_init,
                b = b_init,
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
        // bit0 toggles every rising edge, bit1 toggles when bit0 is 1.
        // bit0 = DFF with d=NOT(bit0), clk=CLK
        // bit1 = DFF with d=XOR(bit0, bit1), clk=CLK
        // XOR(a,b) via NANDs:
        //   nand_ab = NAND(a,b)
        //   nand_a_nab = NAND(a, nand_ab)
        //   nand_b_nab = NAND(b, nand_ab)
        //   xor = NAND(nand_a_nab, nand_b_nab)
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

        // Expected 2-bit counter sequence (bit1:bit0):
        // After each rising edge (odd steps): 00 -> 01 -> 10 -> 11 -> 00 ...
        // Rising edges happen at steps 1, 3, 5, 7, 9, 11, 13, 15.
        let expected_bit0 = [
            false, true, true, false, false, true, true, false, false, true, true, false, false,
            true, true, false, false,
        ];
        let expected_bit1 = [
            false, false, false, true, true, true, true, false, false, false, false, true, true,
            true, true, false, false,
        ];

        for (step, state) in results.iter().enumerate() {
            assert_eq!(
                state["bit0"], expected_bit0[step],
                "bit0 mismatch at step {step}"
            );
            assert_eq!(
                state["bit1"], expected_bit1[step],
                "bit1 mismatch at step {step}"
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
        for step in &results {
            assert!(step.is_empty());
        }
    }

    // ---- Abstraction end-to-end tests ----

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
        for (input, expected) in [(false, true), (true, false)] {
            let yaml = format!(
                r#"
name: abs_not
steps: 0
{prelude}
components:
  - type: dff
    name: x
    init: {input}
  - type: not
    name: inv
    inputs: {{ x: x }}
"#,
                prelude = abs_prelude(),
            );
            let results = run(&yaml);
            assert_eq!(
                results[0]["inv.out"], expected,
                "NOT({input}) should be {expected}"
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
        // Same pattern as raw toggle flip-flop.
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

        for (step, state) in results.iter().enumerate() {
            assert_eq!(
                state["bit0"], expected_bit0[step],
                "bit0 mismatch at step {step}"
            );
            assert_eq!(
                state["bit1"], expected_bit1[step],
                "bit1 mismatch at step {step}"
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
        // Step 0: my_reg/ff starts at 0 (default init)
        assert_eq!(results[0]["my_reg.q"], false);
        // Step 1: rising edge, latches d=one=1
        assert_eq!(results[1]["my_reg.q"], true);
        // Stays 1
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
        // Full adder: sum = a XOR b XOR cin, cout = (a AND b) OR (cin AND (a XOR b))
        let yaml_template = |a: bool, b: bool, cin: bool| {
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

        // Truth table for full adder.
        for (a, b, cin, sum, cout) in [
            (false, false, false, false, false),
            (false, false, true, true, false),
            (false, true, false, true, false),
            (false, true, true, false, true),
            (true, false, false, true, false),
            (true, false, true, false, true),
            (true, true, false, false, true),
            (true, true, true, true, true),
        ] {
            let results = run(&yaml_template(a, b, cin));
            assert_eq!(
                results[0]["fa.sum"], sum,
                "full_add({a},{b},{cin}).sum should be {sum}"
            );
            assert_eq!(
                results[0]["fa.cout"], cout,
                "full_add({a},{b},{cin}).cout should be {cout}"
            );
        }
    }
}
