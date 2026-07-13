use std::collections::HashMap;

use crate::project::PortRef;

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

fn engine_inputs(e: &Editor, flat: &str) -> HashMap<String, String> {
    let def = crate::convert::project_to_circuit(&e.project).unwrap();
    let ex = crate::expander::expand(&def).unwrap();
    crate::compiler::compile(&ex.def).unwrap();
    ex.def
        .components
        .iter()
        .find(|c| c.name == flat)
        .unwrap_or_else(|| panic!("no {flat} in {:?}",
            ex.def.components.iter().map(|c| &c.name).collect::<Vec<_>>()))
        .inputs
        .clone()
}

#[test]
fn bundle_carries_lanes_from_bundler_to_unbundler() {
    let mut e = Editor::empty();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("const", [0, 4]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.place_component("unbundle", [12, 0]).unwrap();
    e.place_component("nand", [18, 0]).unwrap();

    e.connect(port("dff_1", "out"), port("bundle_1", "0"), &[]).unwrap();
    e.connect(port("const_1", "out"), port("bundle_1", "5"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[]).unwrap();
    assert!(
        e.project.wire_nets.iter().any(|n| n.bundle),
        "the bundle wire is flagged"
    );
    e.connect(port("unbundle_1", "0"), port("nand_1", "a"), &[]).unwrap();
    e.connect(port("unbundle_1", "5"), port("nand_1", "b"), &[]).unwrap();

    // The engine sees the lanes wired straight through.
    let inputs = engine_inputs(&e, "nand_1");
    assert_eq!(inputs.get("a"), Some(&"dff_1".to_string()));
    assert_eq!(inputs.get("b"), Some(&"const_1".to_string()));
}

#[test]
fn bundles_and_wires_never_mix() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.place_component("unbundle", [12, 0]).unwrap();

    assert_eq!(
        e.connect(port("bundle_1", "bundle"), port("nand_1", "a"), &[]).unwrap_err(),
        EditError::MixedWireKinds
    );
    assert_eq!(
        e.connect(port("nand_1", "out"), port("unbundle_1", "bundle"), &[]).unwrap_err(),
        EditError::MixedWireKinds
    );
    // Rolled back both times.
    assert!(e.project.wire_nets.is_empty());
}

#[test]
fn bundle_widths_must_match_and_stay_sane() {
    let mut e = Editor::empty();
    e.place_component("bundle", [0, 0]).unwrap();
    e.place_component("unbundle", [6, 0]).unwrap();
    assert_eq!(
        e.set_bundle_width("bundle_1", 1).unwrap_err(),
        EditError::BadBundleWidth
    );
    e.set_bundle_width("unbundle_1", 4).unwrap();
    assert_eq!(
        e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[])
            .unwrap_err(),
        EditError::BundleWidthMismatch(8, 4)
    );
    // Matching them up works.
    e.set_bundle_width("bundle_1", 4).unwrap();
    e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[]).unwrap();
}

#[test]
fn shrinking_a_bundler_scrubs_lost_lanes_and_its_bundle_wire() {
    let mut e = Editor::empty();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.place_component("unbundle", [12, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("bundle_1", "7"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[]).unwrap();

    e.set_bundle_width("bundle_1", 4).unwrap();
    // The lane-7 wire and the (now mismatched) bundle wire are gone.
    assert!(
        e.project.wire_nets.iter().all(|n| n.segments.is_empty() || !n.bundle),
        "bundle wire disconnected"
    );
    assert!(!e
        .project
        .wire_nets
        .iter()
        .any(|n| n.sinks.iter().any(|s| s.port_name == "7")));
    // One undo restores everything.
    e.undo().unwrap();
    assert!(e.project.wire_nets.iter().any(|n| n.bundle));
}

#[test]
fn bundle_pins_cross_abstraction_boundaries() {
    let mut e = Editor::empty();
    // An abstraction taking an 8-line bus: nand(d0, d5) inside.
    let i = e.create_abstraction("bus_nand").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "d", Some(8)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("unbundle", [4, 0]).unwrap();
    e.place_component("nand", [10, 2]).unwrap();
    e.connect(pin("INPUT:", "d"), port("unbundle_1", "bundle"), &[]).unwrap();
    e.connect(port("unbundle_1", "0"), port("nand_1", "a"), &[]).unwrap();
    e.connect(port("unbundle_1", "5"), port("nand_1", "b"), &[]).unwrap();
    e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[]).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();

    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("const", [0, 4]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.place_component("bus_nand", [12, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("bundle_1", "0"), &[]).unwrap();
    e.connect(port("const_1", "out"), port("bundle_1", "5"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("bus_nand_1", "d"), &[]).unwrap();

    // The flattened gate reads the top-level lane drivers directly.
    let inputs = engine_inputs(&e, "bus_nand_1/nand_1");
    assert_eq!(inputs.get("a"), Some(&"dff_1".to_string()));
    assert_eq!(inputs.get("b"), Some(&"const_1".to_string()));
}

#[test]
fn bundle_output_pins_forward_lanes_out() {
    let mut e = Editor::empty();
    // An abstraction producing an 8-line bus from two internal sources.
    let i = e.create_abstraction("two_bits").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", Some(8)).unwrap();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("const", [0, 4]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("bundle_1", "1"), &[]).unwrap();
    e.connect(port("const_1", "out"), port("bundle_1", "2"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), pin("OUTPUT:", "q"), &[]).unwrap();
    // Bundle output pins connect through a REAL bundle net.
    assert!(e.project.abstractions[i].wire_nets.iter().any(|n| n.bundle
        && n.sinks.iter().any(|s| s.component_id == "OUTPUT:q")));
    e.open_sheet(SheetRef::Top).unwrap();

    e.place_component("two_bits", [0, 0]).unwrap();
    e.place_component("unbundle", [8, 0]).unwrap();
    e.place_component("nand", [14, 0]).unwrap();
    e.connect(port("two_bits_1", "q"), port("unbundle_1", "bundle"), &[]).unwrap();
    e.connect(port("unbundle_1", "1"), port("nand_1", "a"), &[]).unwrap();
    e.connect(port("unbundle_1", "2"), port("nand_1", "b"), &[]).unwrap();

    let inputs = engine_inputs(&e, "nand_1");
    assert_eq!(inputs.get("a"), Some(&"two_bits_1/dff_1".to_string()));
    assert_eq!(inputs.get("b"), Some(&"two_bits_1/const_1".to_string()));
}

#[test]
fn a_loop_through_a_bundle_is_refused() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.place_component("unbundle", [12, 0]).unwrap();
    e.connect(port("nand_1", "out"), port("bundle_1", "0"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[]).unwrap();
    // Closing the loop: unbundle lane 0 back into the nand.
    assert_eq!(
        e.connect(port("unbundle_1", "0"), port("nand_1", "a"), &[]).unwrap_err(),
        EditError::CombinationalLoop
    );
}

#[test]
fn rebundling_after_unbundling_chains_through() {
    let mut e = Editor::empty();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("bundle", [4, 0]).unwrap();
    e.place_component("unbundle", [8, 0]).unwrap();
    e.place_component("bundle", [12, 0]).unwrap();
    e.place_component("unbundle", [16, 0]).unwrap();
    e.place_component("nand", [20, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("bundle_1", "3"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[]).unwrap();
    // The legal way to regroup: unbundle, then bundle again.
    e.connect(port("unbundle_1", "3"), port("bundle_2", "6"), &[]).unwrap();
    e.connect(port("bundle_2", "bundle"), port("unbundle_2", "bundle"), &[]).unwrap();
    e.connect(port("unbundle_2", "6"), port("nand_1", "a"), &[]).unwrap();

    let inputs = engine_inputs(&e, "nand_1");
    assert_eq!(inputs.get("a"), Some(&"dff_1".to_string()));
}
