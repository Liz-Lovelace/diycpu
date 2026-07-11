//! Run-mode simulation session: compiles the project once, then lets the
//! user scrub back and forth through clock steps. States are recomputed by
//! replaying from step 0 — trivially cheap at toy scale and stateless, so
//! stepping backwards is free.

use std::collections::HashMap;

use diycpu::compiler::{compile, CompiledCircuit};
use diycpu::convert::project_to_circuit;
use diycpu::expander::expand;
use diycpu::project::{PortRef, Project};
use diycpu::simulator::Simulator;

pub struct SimSession {
    compiled: CompiledCircuit,
    aliases: HashMap<String, String>,
    /// states[k] = component values after k clock toggles (0 = initial).
    states: Vec<HashMap<String, bool>>,
    pub cursor: usize,
}

impl SimSession {
    /// Compile the project and precompute its configured number of steps.
    /// Errors are returned as user-facing status-bar strings.
    pub fn new(project: &Project) -> Result<Self, String> {
        let def = project_to_circuit(project).map_err(|e| e.to_string())?;
        let expanded = expand(&def).map_err(|e| format!("{e:?}"))?;
        let compiled = compile(&expanded.def).map_err(|e| e.to_string())?;
        let mut session = SimSession {
            compiled,
            aliases: expanded.aliases,
            states: Vec::new(),
            cursor: 0,
        };
        let precompute = project.steps.max(1);
        session.compute_states(precompute);
        Ok(session)
    }

    fn compute_states(&mut self, steps: usize) {
        let mut simulator = Simulator::new(&self.compiled);
        let mut states = Vec::with_capacity(steps + 1);
        states.push(self.finish_state(simulator.current_state()));
        for _ in 0..steps {
            simulator.step();
            states.push(self.finish_state(simulator.current_state()));
        }
        self.states = states;
    }

    /// Inject dot-notation aliases so abstraction outputs are addressable.
    fn finish_state(&self, mut state: HashMap<String, bool>) -> HashMap<String, bool> {
        for (alias, target) in &self.aliases {
            if let Some(&value) = state.get(target) {
                state.insert(alias.clone(), value);
            }
        }
        state
    }

    pub fn last_step(&self) -> usize {
        self.states.len() - 1
    }

    pub fn step_forward(&mut self) {
        if self.cursor == self.last_step() {
            // Extend the timeline on demand.
            let new_len = self.states.len() * 2;
            self.compute_states(new_len);
        }
        self.cursor += 1;
    }

    pub fn step_back(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn rewind(&mut self) {
        self.cursor = 0;
    }

    fn state(&self) -> &HashMap<String, bool> {
        &self.states[self.cursor]
    }

    /// Current value of one component output port at the cursor step.
    /// Primitives are keyed by component id, abstraction instance outputs
    /// by `id.port` via the expander's aliases.
    pub fn port_value(&self, is_primitive: bool, port: &PortRef) -> Option<bool> {
        let key = if is_primitive {
            port.component_id.clone()
        } else {
            format!("{}.{}", port.component_id, port.port_name)
        };
        self.state().get(&key).copied()
    }

    /// Value of a component's (single) output, for primitives.
    pub fn component_value(&self, component_id: &str) -> Option<bool> {
        self.state().get(component_id).copied()
    }
}
