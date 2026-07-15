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
fn shrinking_a_bundler_keeps_the_bundle_wire_as_a_flagged_mismatch() {
    let mut e = Editor::empty();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("const", [0, 4]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.place_component("unbundle", [12, 0]).unwrap();
    e.place_component("nand", [18, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("bundle_1", "2"), &[]).unwrap();
    e.connect(port("const_1", "out"), port("bundle_1", "7"), &[]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("unbundle_1", "bundle"), &[]).unwrap();
    e.connect(port("unbundle_1", "2"), port("nand_1", "a"), &[]).unwrap();
    e.connect(port("unbundle_1", "5"), port("nand_1", "b"), &[]).unwrap();

    e.set_bundle_width("bundle_1", 4).unwrap();
    // Lane 7's wire went with its port; the bundle wire itself SURVIVES
    // as a width mismatch (4 → 8) — flagged in the UI, not destroyed.
    assert!(
        e.project.wire_nets.iter().any(|n| n.bundle && !n.segments.is_empty()),
        "the bundle wire stays connected"
    );
    assert!(!e
        .project
        .wire_nets
        .iter()
        .any(|n| n.sinks.iter().any(|s| s.port_name == "7")));

    // And the mismatched connection still WORKS, lanes matched from 0:
    // lane 2 flows; lane 5 (beyond the 4-wide source) reads 0.
    let inputs = engine_inputs(&e, "nand_1");
    assert_eq!(inputs.get("a"), Some(&"dff_1".to_string()));
    assert_eq!(inputs.get("b"), None, "an out-of-range lane is unconnected");
}

#[test]
fn pin_width_changes_keep_wires_too() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("bus").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "d", Some(8)).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("bundle", [0, 0]).unwrap();
    e.place_component("bus", [6, 0]).unwrap();
    e.connect(port("bundle_1", "bundle"), port("bus_1", "d"), &[]).unwrap();

    // Widening the pin keeps the (now mismatched) wire.
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.set_pin_bundle_width("d", Some(16)).unwrap();
    assert!(
        e.project.wire_nets.iter().any(|n| n.bundle && !n.segments.is_empty()),
        "wire survived the width change"
    );
    // Turning it into a SIGNAL pin is a kind change: now the wire goes.
    e.set_pin_bundle_width("d", None).unwrap();
    assert!(
        !e.project.wire_nets.iter().any(|n| !n.segments.is_empty()),
        "kind change disconnects"
    );
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


#[test]
fn undriven_output_pin_lanes_read_zero() {
    let mut e = Editor::empty();
    // A 16-wide output pin fed by an 8-wide bundler: lanes 8.. must exist
    // (as zeros) so consumers of the full 16 lanes still compile.
    let i = e.create_abstraction("wide").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", Some(16)).unwrap();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("bundle", [6, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("bundle_1", "0"), &[]).unwrap();
    // Connect 8-wide bundler to the 16-wide pin: refused as a NEW join…
    assert_eq!(
        e.connect(port("bundle_1", "bundle"), pin("OUTPUT:", "q"), &[]).unwrap_err(),
        EditError::BundleWidthMismatch(8, 16)
    );
    // …so match first, then shrink: the mismatch ARISES and is kept.
    e.set_bundle_width("bundle_1", 16).unwrap();
    e.connect(port("bundle_1", "bundle"), pin("OUTPUT:", "q"), &[]).unwrap();
    e.set_bundle_width("bundle_1", 8).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();

    e.place_component("wide", [0, 0]).unwrap();
    e.place_component("unbundle", [8, 0]).unwrap();
    e.set_bundle_width("unbundle_1", 16).unwrap();
    e.place_component("nand", [14, 0]).unwrap();
    e.connect(port("wide_1", "q"), port("unbundle_1", "bundle"), &[]).unwrap();
    e.connect(port("unbundle_1", "0"), port("nand_1", "a"), &[]).unwrap();
    e.connect(port("unbundle_1", "12"), port("nand_1", "b"), &[]).unwrap();

    let inputs = engine_inputs(&e, "nand_1");
    assert_eq!(inputs.get("a"), Some(&"wide_1/dff_1".to_string()));
    assert_eq!(
        inputs.get("b"),
        Some(&"wide_1/__zero_q__b12".to_string()),
        "an undriven lane presents a constant 0"
    );
}


#[test]
fn resizes_that_would_overlap_a_neighbour_are_refused() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("box").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "a", None).unwrap();
    e.set_abstraction_size(i, 4.0, 2.0).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();
    // Two instances side by side with a one-cell gap, and a nand snug
    // under the first.
    e.place_component("box", [0, 0]).unwrap(); // 4×2
    e.place_component("box", [5, 0]).unwrap();
    e.place_component("nand", [0, 3]).unwrap();

    // Growing the definition would push instance 1 into instance 2.
    assert_eq!(
        e.set_abstraction_size(i, 6.0, 2.0).unwrap_err(),
        EditError::Overlap
    );
    assert_eq!(e.project.abstractions[i].width, 4.0, "rolled back");
    // Growing DOWN collides with the nand.
    assert_eq!(
        e.set_abstraction_size(i, 4.0, 4.0).unwrap_err(),
        EditError::Overlap
    );
    // A size that fits everywhere is fine.
    e.set_abstraction_size(i, 4.0, 2.5).unwrap();

    // Adding pins until the body must grow: refused at the same wall.
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    let mut grew_and_hit_the_wall = false;
    for k in 0..8 {
        match e.add_interface_pin(PortKind::Input, &format!("p{k}"), None) {
            Ok(()) => {}
            Err(EditError::Overlap) => {
                grew_and_hit_the_wall = true;
                break;
            }
            Err(other) => panic!("unexpected: {other}"),
        }
    }
    assert!(grew_and_hit_the_wall, "auto-grow respects neighbours");
}
