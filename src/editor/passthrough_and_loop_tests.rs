
use crate::project::{PortRef, Project};

use super::*;
use crate::layout::PortKind;

fn port(c: &str, p: &str) -> PortRef {
    PortRef {
        component_id: c.into(),
        port_name: p.into(),
    }
}
fn pin(prefix: &str, name: &str) -> PortRef {
    PortRef {
        component_id: format!("{prefix}{name}"),
        port_name: name.into(),
    }
}

/// A pure wire abstraction: output q forwards input x, no components.
fn make_buf(e: &mut Editor) -> usize {
    let i = e.create_abstraction("buf").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.connect(pin("INPUT:", "x"), pin("OUTPUT:", "q"), &[]).unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("q"),
        Some(&"INPUT:x".to_string())
    );
    e.open_sheet(SheetRef::Top).unwrap();
    i
}

#[test]
fn passthrough_resolves_through_the_engine() {
    let mut e = Editor::empty();
    make_buf(&mut e);
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("buf", [6, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("buf_1", "x"), &[]).unwrap();

    let def = crate::convert::project_to_circuit(&e.project).unwrap();
    let expanded = crate::expander::expand(&def).unwrap();
    // The instance's output IS the signal wired into its input.
    assert_eq!(expanded.aliases.get("buf_1.q"), Some(&"dff_1".to_string()));
    crate::compiler::compile(&expanded.def).unwrap();
}

#[test]
fn passthrough_chains_across_nesting_and_consumers() {
    let mut e = Editor::empty();
    make_buf(&mut e);
    // buf2 wraps buf: r forwards y through the nested passthrough.
    let i2 = e.create_abstraction("buf2").unwrap();
    e.open_sheet(SheetRef::Abstraction(i2)).unwrap();
    e.add_interface_pin(PortKind::Input, "y", None).unwrap();
    e.add_interface_pin(PortKind::Output, "r", None).unwrap();
    e.place_component("buf", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "y"), port("buf_1", "x"), &[]).unwrap();
    e.connect(port("buf_1", "q"), pin("OUTPUT:", "r"), &[]).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();

    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("buf2", [6, 0]).unwrap();
    e.place_component("nand", [12, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("buf2_1", "y"), &[]).unwrap();
    e.connect(port("buf2_1", "r"), port("nand_1", "a"), &[]).unwrap();

    let def = crate::convert::project_to_circuit(&e.project).unwrap();
    let expanded = crate::expander::expand(&def).unwrap();
    assert_eq!(
        expanded.aliases.get("buf2_1.r"),
        Some(&"dff_1".to_string()),
        "the alias climbs through both passthrough levels"
    );
    // The consumer's input followed the whole alias chain to the flat
    // driver.
    let nand = expanded
        .def
        .components
        .iter()
        .find(|c| c.name == "nand_1")
        .unwrap();
    assert_eq!(nand.inputs.get("a"), Some(&"dff_1".to_string()));
    crate::compiler::compile(&expanded.def).unwrap();
}

#[test]
fn combinational_loops_are_refused_when_wired() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    // Straight self-loop.
    assert_eq!(
        e.connect(port("nand_1", "out"), port("nand_1", "a"), &[]).unwrap_err(),
        EditError::CombinationalLoop
    );
    assert!(e.project.wire_nets.is_empty(), "the refused wire rolled back");

    // A loop through a second gate.
    e.place_component("nand", [6, 0]).unwrap();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[]).unwrap();
    assert_eq!(
        e.connect(port("nand_2", "out"), port("nand_1", "a"), &[]).unwrap_err(),
        EditError::CombinationalLoop
    );

    // The same loop with a flip-flop in it is a legitimate register.
    e.place_component("dff", [12, 0]).unwrap();
    e.connect(port("nand_2", "out"), port("dff_1", "d"), &[]).unwrap();
    e.connect(port("dff_1", "out"), port("nand_1", "a"), &[]).unwrap();
}

#[test]
fn loops_are_refused_inside_an_uninstantiated_abstraction() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("loopy").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.place_component("nand", [0, 0]).unwrap();
    // No instance of "loopy" exists anywhere, but the loop is still
    // caught (the guard probes the sheet as its own instance).
    assert_eq!(
        e.connect(port("nand_1", "out"), port("nand_1", "b"), &[]).unwrap_err(),
        EditError::CombinationalLoop
    );
    assert!(e.project.abstractions[i].wire_nets.is_empty());
}

#[test]
fn renaming_and_removing_a_forwarded_input_updates_the_output() {
    let mut e = Editor::empty();
    let i = make_buf(&mut e);
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.rename_interface_pin(PortKind::Input, "x", "z").unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("q"),
        Some(&"INPUT:z".to_string())
    );
    e.remove_interface_pin(PortKind::Input, "z").unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("q"),
        Some(&String::new()),
        "the forwarding output became unwired"
    );
}

#[test]
fn normalize_keeps_valid_passthroughs_and_clears_ghosts() {
    let json = r#"{
        "name": "t", "steps": 1, "components": [],
        "abstractions": [{
            "name": "buf", "inputs": ["x"],
            "outputs": {"q": "INPUT:x", "r": "INPUT:ghost"},
            "components": []
        }]
    }"#;
    let mut project: Project = serde_json::from_str(json).unwrap();
    let notes = normalize(&mut project);
    assert_eq!(
        project.abstractions[0].outputs.get("q"),
        Some(&"INPUT:x".to_string()),
        "a declared input stays forwarded"
    );
    assert_eq!(project.abstractions[0].outputs.get("r"), Some(&String::new()));
    assert!(notes.iter().any(|n| n.contains("ghost")));
}
