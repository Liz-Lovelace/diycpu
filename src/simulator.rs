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
        // Every clk component carries the same shared signal: they all
        // start at 0 and toggle together.
        for &clock_index in &self.circuit.clock_indices {
            self.component_values[clock_index] = !self.component_values[clock_index];
        }
    }

    /// Check each DFF for a 0→1 (rising) edge on its clock input. If found,
    /// latch the current data-input value. Returns true if any DFF latched.
    ///
    /// All DFF data inputs are read first (using pre-latch values), then all
    /// outputs are updated simultaneously. This ensures a chain of DFFs on the
    /// same clock edge behaves like a shift register, not a cascade.
    fn latch_dffs_on_rising_edges(&mut self) -> bool {
        // Phase 1: read all data inputs using the current (pre-latch) values.
        let mut pending_latches: Vec<(usize, bool)> = Vec::new();

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
                pending_latches.push((component_index, data_value));
            }
        }

        // Phase 2: apply all latched values simultaneously.
        let any_latched = !pending_latches.is_empty();
        for (component_index, data_value) in pending_latches {
            self.component_values[component_index] = data_value;
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
            // Unconnected inputs read as constant 0.
            let a = input_a.map(|i| component_values[i]).unwrap_or(false);
            let b = input_b.map(|i| component_values[i]).unwrap_or(false);
            component_values[component_index] = !(a && b);
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
    use crate::test_helpers::{abs_prelude, AbstractionBuilder, CircuitBuilder};
    use crate::types::CircuitDef;

    /// Expand abstractions, compile, simulate, and inject output aliases.
    fn run(def: CircuitDef) -> Vec<HashMap<String, bool>> {
        let expanded = expand(&def).unwrap();
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
        let def = CircuitBuilder::new("constant", 4)
            .dff("high").init(true).done()
            .dff("low").init(false).done()
            .build();
        let results = run(def);
        for step_state in &results {
            assert!(step_state["high"]);
            assert!(!step_state["low"]);
        }
    }

    #[test]
    fn not_gate() {
        let def = CircuitBuilder::new("not", 2)
            .dff("input").init(true).done()
            .nand("output", "input", "input")
            .build();
        let results = run(def);
        // NAND(1,1) = 0
        for step_state in &results {
            assert!(step_state["input"]);
            assert!(!step_state["output"]);
        }
    }

    #[test]
    fn toggle_flip_flop() {
        let def = CircuitBuilder::new("toggle", 8)
            .clk("clk")
            .dff("state").init(false).d("not_state").clk("clk").done()
            .nand("not_state", "state", "state")
            .build();
        let results = run(def);

        assert!(!results[0]["clk"]);
        assert!(!results[0]["state"]);
        assert!(results[0]["not_state"]);

        assert!(results[1]["clk"]);
        assert!(results[1]["state"]);
        assert!(!results[1]["not_state"]);

        assert!(!results[2]["clk"]);
        assert!(results[2]["state"]);
        assert!(!results[2]["not_state"]);

        assert!(results[3]["clk"]);
        assert!(!results[3]["state"]);
        assert!(results[3]["not_state"]);

        assert!(!results[4]["state"]);
        assert!(results[5]["state"]);
        assert!(results[6]["state"]);
        assert!(!results[7]["state"]);
    }

    #[test]
    fn and_gate() {
        let def = CircuitBuilder::new("and", 0)
            .dff("a").init(true).done()
            .dff("b").init(true).done()
            .nand("nand_ab", "a", "b")
            .nand("and_ab", "nand_ab", "nand_ab")
            .build();
        let results = run(def);
        assert!(results[0]["and_ab"]);
    }

    #[test]
    fn and_gate_false_cases() {
        let def = CircuitBuilder::new("and_10", 0)
            .dff("a").init(true).done()
            .dff("b").init(false).done()
            .nand("nand_ab", "a", "b")
            .nand("and_ab", "nand_ab", "nand_ab")
            .build();
        let results = run(def);
        assert!(!results[0]["and_ab"]);
    }

    #[test]
    fn or_gate() {
        let def = CircuitBuilder::new("or", 0)
            .dff("a").init(false).done()
            .dff("b").init(true).done()
            .nand("not_a", "a", "a")
            .nand("not_b", "b", "b")
            .nand("or_ab", "not_a", "not_b")
            .build();
        let results = run(def);
        assert!(results[0]["or_ab"]);
    }

    #[test]
    fn or_gate_both_false() {
        let def = CircuitBuilder::new("or_00", 0)
            .dff("a").init(false).done()
            .dff("b").init(false).done()
            .nand("not_a", "a", "a")
            .nand("not_b", "b", "b")
            .nand("or_ab", "not_a", "not_b")
            .build();
        let results = run(def);
        assert!(!results[0]["or_ab"]);
    }

    #[test]
    fn no_clk_circuit() {
        let def = CircuitBuilder::new("no_clk", 3)
            .dff("one").init(true).done()
            .dff("zero").init(false).done()
            .nand("result", "one", "zero")
            .build();
        let results = run(def);
        for step_state in &results {
            assert!(step_state["result"]);
        }
    }

    #[test]
    fn dff_d_without_clk_never_latches() {
        let def = CircuitBuilder::new("no_latch", 4)
            .dff("source").init(true).done()
            .dff("target").init(false).d("source").done()
            .build();
        let results = run(def);
        for step_state in &results {
            assert!(!step_state["target"]);
        }
    }

    #[test]
    fn rising_edge_only() {
        let def = CircuitBuilder::new("edge_test", 6)
            .clk("clk")
            .dff("one").init(true).done()
            .dff("reg").init(false).d("one").clk("clk").done()
            .build();
        let results = run(def);
        assert!(!results[0]["reg"]);
        assert!(results[1]["reg"]);
        assert!(results[2]["reg"]);
        assert!(results[3]["reg"]);
        assert!(results[4]["reg"]);
        assert!(results[5]["reg"]);
    }

    #[test]
    fn multiple_dffs_independent() {
        let def = CircuitBuilder::new("multi_dff", 4)
            .clk("clk")
            .dff("a").init(true).done()
            .dff("b").init(false).done()
            .nand("not_a", "a", "a")
            .nand("not_b", "b", "b")
            .dff("reg_a").init(false).d("not_a").clk("clk").done()
            .dff("reg_b").init(false).d("not_b").clk("clk").done()
            .build();
        let results = run(def);
        assert!(!results[0]["reg_a"]);
        assert!(!results[0]["reg_b"]);
        assert!(!results[1]["reg_a"]);
        assert!(results[1]["reg_b"]);
    }

    #[test]
    fn dff_self_reference_holds_value() {
        let def = CircuitBuilder::new("self_hold", 4)
            .clk("clk")
            .dff("reg").init(true).d("reg").clk("clk").done()
            .build();
        let results = run(def);
        for step_state in &results {
            assert!(step_state["reg"]);
        }
    }

    #[test]
    fn xor_gate() {
        for (a_init, b_init, expected) in
            [(false, false, false), (false, true, true), (true, false, true), (true, true, false)]
        {
            let def = CircuitBuilder::new("xor", 0)
                .dff("a").init(a_init).done()
                .dff("b").init(b_init).done()
                .nand("nand_ab", "a", "b")
                .nand("nand_a_nab", "a", "nand_ab")
                .nand("nand_b_nab", "b", "nand_ab")
                .nand("xor", "nand_a_nab", "nand_b_nab")
                .build();
            let results = run(def);
            assert_eq!(
                results[0]["xor"], expected,
                "XOR({a_init}, {b_init}) should be {expected}"
            );
        }
    }

    #[test]
    fn two_bit_counter() {
        let def = CircuitBuilder::new("counter", 16)
            .clk("clk")
            .dff("bit0").init(false).d("not_bit0").clk("clk").done()
            .dff("bit1").init(false).d("xor_01").clk("clk").done()
            .nand("not_bit0", "bit0", "bit0")
            .nand("nand_01", "bit0", "bit1")
            .nand("nand_0_n01", "bit0", "nand_01")
            .nand("nand_1_n01", "bit1", "nand_01")
            .nand("xor_01", "nand_0_n01", "nand_1_n01")
            .build();
        let results = run(def);

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
    fn shift_register_propagation() {
        // A chain of DFFs on the same clock: the "1" should propagate one stage
        // per rising edge, NOT flood through all stages at once.
        let def = CircuitBuilder::new("shift_reg", 8)
            .clk("clk")
            .dff("source").init(true).done()
            .dff("stage1").init(false).d("source").clk("clk").done()
            .dff("stage2").init(false).d("stage1").clk("clk").done()
            .dff("stage3").init(false).d("stage2").clk("clk").done()
            .build();
        let results = run(def);

        // Step 0: initial state (CLK=0)
        assert!(!results[0]["stage1"]);
        assert!(!results[0]["stage2"]);
        assert!(!results[0]["stage3"]);

        // Step 1: first rising edge — only stage1 latches "source"=1
        assert!(results[1]["stage1"]);
        assert!(!results[1]["stage2"]);
        assert!(!results[1]["stage3"]);

        // Step 2: falling edge — no change
        assert!(results[2]["stage1"]);
        assert!(!results[2]["stage2"]);
        assert!(!results[2]["stage3"]);

        // Step 3: second rising edge — stage2 latches stage1's OLD value (1)
        assert!(results[3]["stage1"]);
        assert!(results[3]["stage2"]);
        assert!(!results[3]["stage3"]);

        // Step 5: third rising edge — stage3 latches stage2's OLD value (1)
        assert!(results[5]["stage1"]);
        assert!(results[5]["stage2"]);
        assert!(results[5]["stage3"]);
    }

    #[test]
    fn step_count_matches() {
        let def = CircuitBuilder::new("count_test", 5)
            .dff("x").done()
            .build();
        let results = run(def);
        assert_eq!(results.len(), 6);
    }

    #[test]
    fn zero_steps() {
        let def = CircuitBuilder::new("zero", 0)
            .dff("x").init(true).done()
            .build();
        let results = run(def);
        assert_eq!(results.len(), 1);
        assert!(results[0]["x"]);
    }

    #[test]
    fn empty_circuit() {
        let def = CircuitBuilder::new("empty", 3).build();
        let results = run(def);
        assert_eq!(results.len(), 4);
        for step_state in &results {
            assert!(step_state.is_empty());
        }
    }

    // ── Abstraction end-to-end tests ─────────────────────────────────────

    fn with_prelude(steps: usize) -> CircuitBuilder {
        let mut builder = CircuitBuilder::new("test", steps);
        for abs in abs_prelude() {
            builder = builder.abstraction(abs);
        }
        builder
    }

    #[test]
    fn abs_not_gate() {
        for (input_value, expected_output) in [(false, true), (true, false)] {
            let def = with_prelude(0)
                .dff("x").init(input_value).done()
                .component("not", "inv", &[("x", "x")])
                .build();
            let results = run(def);
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
            let def = with_prelude(0)
                .dff("a").init(a).done()
                .dff("b").init(b).done()
                .component("and", "g", &[("a", "a"), ("b", "b")])
                .build();
            let results = run(def);
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
            let def = with_prelude(0)
                .dff("a").init(a).done()
                .dff("b").init(b).done()
                .component("or", "g", &[("a", "a"), ("b", "b")])
                .build();
            let results = run(def);
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
            let def = with_prelude(0)
                .dff("a").init(a).done()
                .dff("b").init(b).done()
                .component("xor", "g", &[("a", "a"), ("b", "b")])
                .build();
            let results = run(def);
            assert_eq!(
                results[0]["g.out"], expected,
                "XOR({a}, {b}) should be {expected}"
            );
        }
    }

    #[test]
    fn abs_toggle_with_not() {
        let def = with_prelude(8)
            .clk("clk")
            .dff("state").init(false).d("inv.out").clk("clk").done()
            .component("not", "inv", &[("x", "state")])
            .build();
        let results = run(def);
        assert!(!results[0]["state"]);
        assert!(results[1]["state"]);
        assert!(results[2]["state"]);
        assert!(!results[3]["state"]);
        assert!(!results[4]["state"]);
        assert!(results[5]["state"]);
    }

    #[test]
    fn abs_counter_with_xor_and_not() {
        let def = with_prelude(16)
            .clk("clk")
            .dff("bit0").init(false).d("n0.out").clk("clk").done()
            .dff("bit1").init(false).d("x01.out").clk("clk").done()
            .component("not", "n0", &[("x", "bit0")])
            .component("xor", "x01", &[("a", "bit0"), ("b", "bit1")])
            .build();
        let results = run(def);

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
        let register_abs = AbstractionBuilder::new("register")
            .inputs(&["d", "clk"])
            .output("q", "ff")
            .dff("ff").d("d").clk("clk").done()
            .build();
        let def = CircuitBuilder::new("dff_in_abs", 4)
            .abstraction(register_abs)
            .clk("clk")
            .dff("one").init(true).done()
            .component("register", "my_reg", &[("d", "one"), ("clk", "clk")])
            .build();
        let results = run(def);
        assert!(!results[0]["my_reg.q"]);
        assert!(results[1]["my_reg.q"]);
        assert!(results[2]["my_reg.q"]);
        assert!(results[3]["my_reg.q"]);
    }

    #[test]
    fn abs_multiple_outputs() {
        let not_abs = AbstractionBuilder::new("not")
            .inputs(&["x"])
            .output("out", "gate")
            .nand("gate", "x", "x")
            .build();
        let split_abs = AbstractionBuilder::new("split")
            .inputs(&["x"])
            .output("normal", "buf.out")
            .output("inverted", "inv.out")
            .component("not", "inv", &[("x", "x")])
            .component("not", "buf", &[("x", "inv.out")])
            .build();
        let def = CircuitBuilder::new("multi_out", 0)
            .abstraction(not_abs)
            .abstraction(split_abs)
            .dff("val").init(true).done()
            .component("split", "s", &[("x", "val")])
            .build();
        let results = run(def);
        assert!(results[0]["s.normal"]);
        assert!(!results[0]["s.inverted"]);
    }

    #[test]
    fn abs_full_adder() {
        let full_add_abs = AbstractionBuilder::new("full_add")
            .inputs(&["a", "b", "cin"])
            .output("sum", "xor2.out")
            .output("cout", "cout_or.out")
            .component("xor", "xor1", &[("a", "a"), ("b", "b")])
            .component("xor", "xor2", &[("a", "xor1.out"), ("b", "cin")])
            .component("and", "and1", &[("a", "xor1.out"), ("b", "cin")])
            .component("and", "and2", &[("a", "a"), ("b", "b")])
            .component("or", "cout_or", &[("a", "and1.out"), ("b", "and2.out")])
            .build();

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
            let mut builder = CircuitBuilder::new("full_adder", 0);
            for abs in abs_prelude() {
                builder = builder.abstraction(abs);
            }
            let def = builder
                .abstraction(full_add_abs.clone())
                .dff("a").init(a).done()
                .dff("b").init(b).done()
                .dff("cin").init(cin).done()
                .component("full_add", "fa", &[("a", "a"), ("b", "b"), ("cin", "cin")])
                .build();
            let results = run(def);
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
