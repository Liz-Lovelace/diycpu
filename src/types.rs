use std::collections::HashMap;
use std::fmt;

/// Top-level circuit definition for the engine.
/// Contains no visual/GUI data — only what's needed for compilation and simulation.
#[derive(Debug, Clone)]
pub struct CircuitDef {
    pub name: String,
    pub steps: usize,
    pub abstractions: Vec<AbstractionDef>,
    pub components: Vec<ComponentDef>,
}

/// An abstraction (reusable subcircuit template).
#[derive(Debug, Clone)]
pub struct AbstractionDef {
    pub name: String,
    pub inputs: Vec<String>,
    /// Maps output_name -> internal_component_or_sub_output reference.
    pub outputs: HashMap<String, String>,
    pub components: Vec<ComponentDef>,
}

/// A single component in a circuit definition.
/// For primitives (clk, nand, dff), the compiler validates the inputs.
/// For abstraction instances, the expander resolves them.
#[derive(Debug, Clone)]
pub struct ComponentDef {
    pub component_type: String,
    pub name: String,
    pub init: Option<bool>,
    /// Maps input port name -> source component reference. Empty map = no inputs.
    pub inputs: HashMap<String, String>,
}

impl ComponentDef {
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ComponentDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.component_type, self.name)
    }
}

pub const PRIMITIVE_TYPES: &[&str] = &["clk", "nand", "dff"];

pub fn is_primitive(type_name: &str) -> bool {
    PRIMITIVE_TYPES.contains(&type_name)
}
