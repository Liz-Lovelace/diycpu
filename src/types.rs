use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;

/// Top-level circuit definition, deserialized from YAML.
#[derive(Debug, Clone, Deserialize)]
pub struct CircuitDef {
    pub name: String,
    pub steps: usize,
    #[serde(default)]
    pub abstractions: Vec<AbstractionDef>,
    pub components: Vec<ComponentDef>,
}

/// An abstraction (reusable subcircuit template).
#[derive(Debug, Clone, Deserialize)]
pub struct AbstractionDef {
    pub name: String,
    pub inputs: Vec<String>,
    /// Maps output_name -> internal_component_or_sub_output reference.
    pub outputs: HashMap<String, String>,
    pub components: Vec<ComponentDef>,
}

/// A single component. For primitives (clk, nand, dff), the compiler validates
/// the inputs. For abstraction instances, the expander resolves them.
#[derive(Debug, Clone, Deserialize)]
pub struct ComponentDef {
    #[serde(rename = "type")]
    pub component_type: String,
    pub name: String,
    #[serde(default, deserialize_with = "option_bool_from_value")]
    pub init: Option<bool>,
    #[serde(default)]
    pub inputs: Option<HashMap<String, String>>,
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

/// Custom deserializer: accepts bool, 0/1 integer, or absent (None).
fn option_bool_from_value<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct Visitor;

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Option<bool>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a boolean or 0/1 integer")
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Option<bool>, E> {
            Ok(Some(v))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Option<bool>, E> {
            match v {
                0 => Ok(Some(false)),
                1 => Ok(Some(true)),
                _ => Err(E::custom(format!("expected 0 or 1, got {v}"))),
            }
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Option<bool>, E> {
            match v {
                0 => Ok(Some(false)),
                1 => Ok(Some(true)),
                _ => Err(E::custom(format!("expected 0 or 1, got {v}"))),
            }
        }

        fn visit_unit<E: de::Error>(self) -> Result<Option<bool>, E> {
            Ok(None)
        }

        fn visit_none<E: de::Error>(self) -> Result<Option<bool>, E> {
            Ok(None)
        }
    }

    deserializer.deserialize_any(Visitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_component_types() {
        let yaml = r#"
name: test
steps: 5
components:
  - type: clk
    name: clock
  - type: nand
    name: gate1
    inputs:
      a: clock
      b: clock
  - type: dff
    name: reg1
    init: 1
    inputs:
      d: gate1
      clk: clock
"#;
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.name, "test");
        assert_eq!(def.steps, 5);
        assert_eq!(def.components.len(), 3);
        assert_eq!(def.components[0].name(), "clock");
        assert_eq!(def.components[1].name(), "gate1");
        assert_eq!(def.components[2].name(), "reg1");
    }

    #[test]
    fn parse_dff_no_inputs() {
        let yaml = r#"
name: test
steps: 1
components:
  - type: dff
    name: constant
    init: true
"#;
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.components[0].component_type, "dff");
        assert_eq!(def.components[0].init, Some(true));
        assert!(def.components[0].inputs.is_none());
    }

    #[test]
    fn parse_dff_partial_inputs() {
        let yaml = r#"
name: test
steps: 1
components:
  - type: dff
    name: reg
    inputs:
      d: something
"#;
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        let inputs = def.components[0].inputs.as_ref().unwrap();
        assert_eq!(inputs.get("d").map(|s| s.as_str()), Some("something"));
        assert!(inputs.get("clk").is_none());
    }

    #[test]
    fn parse_init_as_integer() {
        let yaml = r#"
name: test
steps: 1
components:
  - type: dff
    name: a
    init: 0
  - type: dff
    name: b
    init: 1
"#;
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.components[0].init, Some(false));
        assert_eq!(def.components[1].init, Some(true));
    }

    #[test]
    fn parse_init_defaults_to_none() {
        let yaml = r#"
name: test
steps: 1
components:
  - type: dff
    name: reg
"#;
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.components[0].init, None);
    }

    #[test]
    fn parse_abstractions() {
        let yaml = r#"
name: test
steps: 1
abstractions:
  - name: not
    inputs: [x]
    outputs:
      out: gate
    components:
      - type: nand
        name: gate
        inputs: { a: x, b: x }
components:
  - type: dff
    name: a
    init: true
  - type: not
    name: my_not
    inputs: { x: a }
"#;
        let def: CircuitDef = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(def.abstractions.len(), 1);
        assert_eq!(def.abstractions[0].name, "not");
        assert_eq!(def.abstractions[0].inputs, vec!["x"]);
        assert_eq!(def.abstractions[0].outputs["out"], "gate");
        assert_eq!(def.abstractions[0].components.len(), 1);
        // The instance
        assert_eq!(def.components[1].component_type, "not");
        assert_eq!(def.components[1].name, "my_not");
    }
}
