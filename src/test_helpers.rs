use crate::types::{AbstractionDef, CircuitDef, ComponentDef};
use std::collections::HashMap;

/// Builder for constructing CircuitDef in tests without parsing.
pub struct CircuitBuilder {
    name: String,
    steps: usize,
    abstractions: Vec<AbstractionDef>,
    components: Vec<ComponentDef>,
}

impl CircuitBuilder {
    pub fn new(name: &str, steps: usize) -> Self {
        Self {
            name: name.into(),
            steps,
            abstractions: vec![],
            components: vec![],
        }
    }

    pub fn clk(mut self, name: &str) -> Self {
        self.components.push(ComponentDef {
            component_type: "clk".into(),
            name: name.into(),
            init: None,
            inputs: HashMap::new(),
        });
        self
    }

    pub fn nand(mut self, name: &str, a: &str, b: &str) -> Self {
        let mut inputs = HashMap::new();
        inputs.insert("a".into(), a.into());
        inputs.insert("b".into(), b.into());
        self.components.push(ComponentDef {
            component_type: "nand".into(),
            name: name.into(),
            init: None,
            inputs,
        });
        self
    }

    pub fn dff(self, name: &str) -> DffBuilder {
        DffBuilder {
            circuit: self,
            name: name.into(),
            init: None,
            input_d: None,
            input_clk: None,
        }
    }

    /// Add a generic component (e.g. abstraction instance) with named inputs.
    pub fn component(mut self, comp_type: &str, name: &str, inputs: &[(&str, &str)]) -> Self {
        let input_map: HashMap<String, String> = inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.components.push(ComponentDef {
            component_type: comp_type.into(),
            name: name.into(),
            init: None,
            inputs: input_map,
        });
        self
    }

    pub fn abstraction(mut self, abs: AbstractionDef) -> Self {
        self.abstractions.push(abs);
        self
    }

    pub fn build(self) -> CircuitDef {
        CircuitDef {
            name: self.name,
            steps: self.steps,
            abstractions: self.abstractions,
            components: self.components,
        }
    }
}

pub struct DffBuilder {
    circuit: CircuitBuilder,
    name: String,
    init: Option<bool>,
    input_d: Option<String>,
    input_clk: Option<String>,
}

impl DffBuilder {
    pub fn init(mut self, value: bool) -> Self {
        self.init = Some(value);
        self
    }

    pub fn d(mut self, source: &str) -> Self {
        self.input_d = Some(source.into());
        self
    }

    pub fn clk(mut self, source: &str) -> Self {
        self.input_clk = Some(source.into());
        self
    }

    pub fn done(mut self) -> CircuitBuilder {
        let mut inputs = HashMap::new();
        if let Some(d) = self.input_d {
            inputs.insert("d".into(), d);
        }
        if let Some(clk) = self.input_clk {
            inputs.insert("clk".into(), clk);
        }
        self.circuit.components.push(ComponentDef {
            component_type: "dff".into(),
            name: self.name,
            init: self.init,
            inputs,
        });
        self.circuit
    }
}

/// Builder for constructing AbstractionDef in tests.
pub struct AbstractionBuilder {
    name: String,
    inputs: Vec<String>,
    outputs: HashMap<String, String>,
    components: Vec<ComponentDef>,
}

impl AbstractionBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            inputs: vec![],
            outputs: HashMap::new(),
            components: vec![],
        }
    }

    pub fn inputs(mut self, names: &[&str]) -> Self {
        self.inputs = names.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn output(mut self, name: &str, target: &str) -> Self {
        self.outputs.insert(name.into(), target.into());
        self
    }

    pub fn nand(mut self, name: &str, a: &str, b: &str) -> Self {
        let mut inputs = HashMap::new();
        inputs.insert("a".into(), a.into());
        inputs.insert("b".into(), b.into());
        self.components.push(ComponentDef {
            component_type: "nand".into(),
            name: name.into(),
            init: None,
            inputs,
        });
        self
    }

    pub fn dff(self, name: &str) -> AbstractionDffBuilder {
        AbstractionDffBuilder {
            abs: self,
            name: name.into(),
            init: None,
            input_d: None,
            input_clk: None,
        }
    }

    /// Add a generic component inside the abstraction.
    pub fn component(mut self, comp_type: &str, name: &str, inputs: &[(&str, &str)]) -> Self {
        let input_map: HashMap<String, String> = inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.components.push(ComponentDef {
            component_type: comp_type.into(),
            name: name.into(),
            init: None,
            inputs: input_map,
        });
        self
    }

    pub fn build(self) -> AbstractionDef {
        AbstractionDef {
            name: self.name,
            inputs: self.inputs,
            outputs: self.outputs,
            components: self.components,
        }
    }
}

pub struct AbstractionDffBuilder {
    abs: AbstractionBuilder,
    name: String,
    init: Option<bool>,
    input_d: Option<String>,
    input_clk: Option<String>,
}

impl AbstractionDffBuilder {
    pub fn init(mut self, value: bool) -> Self {
        self.init = Some(value);
        self
    }

    pub fn d(mut self, source: &str) -> Self {
        self.input_d = Some(source.into());
        self
    }

    pub fn clk(mut self, source: &str) -> Self {
        self.input_clk = Some(source.into());
        self
    }

    pub fn done(mut self) -> AbstractionBuilder {
        let mut inputs = HashMap::new();
        if let Some(d) = self.input_d {
            inputs.insert("d".into(), d);
        }
        if let Some(clk) = self.input_clk {
            inputs.insert("clk".into(), clk);
        }
        self.abs.components.push(ComponentDef {
            component_type: "dff".into(),
            name: self.name,
            init: self.init,
            inputs,
        });
        self.abs
    }
}

/// Common abstraction prelude: not, and, or, xor gates built from NANDs.
pub fn abs_prelude() -> Vec<AbstractionDef> {
    vec![
        AbstractionBuilder::new("not")
            .inputs(&["x"])
            .output("out", "gate")
            .nand("gate", "x", "x")
            .build(),
        AbstractionBuilder::new("and")
            .inputs(&["a", "b"])
            .output("out", "inv.out")
            .nand("nab", "a", "b")
            .component("not", "inv", &[("x", "nab")])
            .build(),
        AbstractionBuilder::new("or")
            .inputs(&["a", "b"])
            .output("out", "result")
            .component("not", "na", &[("x", "a")])
            .component("not", "nb", &[("x", "b")])
            .nand("result", "na.out", "nb.out")
            .build(),
        AbstractionBuilder::new("xor")
            .inputs(&["a", "b"])
            .output("out", "result")
            .nand("nab", "a", "b")
            .nand("na_nab", "a", "nab")
            .nand("nb_nab", "b", "nab")
            .nand("result", "na_nab", "nb_nab")
            .build(),
    ]
}
