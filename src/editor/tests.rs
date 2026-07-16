use std::collections::HashMap;

use crate::layout::{self, PortKind};
use crate::project::{Endpoint, PortRef, Project, ProjectComponent, WireNet};

use super::*;
use crate::project::ProjectAbstraction;

fn port(c: &str, p: &str) -> PortRef {
    PortRef {
        component_id: c.into(),
        port_name: p.into(),
    }
}

fn editor_with_two_nands() -> Editor {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    e.place_component("nand", [10, 0]).unwrap();
    e
}

#[test]
fn place_generates_sequential_ids() {
    let mut e = Editor::empty();
    assert_eq!(e.place_component("nand", [0, 0]).unwrap(), "nand_1");
    assert_eq!(e.place_component("nand", [10, 0]).unwrap(), "nand_2");
    assert_eq!(e.place_component("dff", [20, 0]).unwrap(), "dff_1");
    // dff gets a default init so the checkbox has a value.
    assert_eq!(e.project.components[2].init, Some(false));
}

#[test]
fn place_rejects_overlap_and_unknown_type() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    assert_eq!(
        e.place_component("nand", [1, 1]).unwrap_err(),
        EditError::Overlap
    );
    // Touching edge-to-edge is allowed.
    e.place_component("nand", [2, 0]).unwrap();
    assert!(matches!(
        e.place_component("bogus", [20, 20]).unwrap_err(),
        EditError::UnknownComponentType(_)
    ));
    // Failed ops must not leave undo steps or mutations.
    assert_eq!(e.project.components.len(), 2);
}

#[test]
fn clocks_are_a_shared_signal() {
    // Several clk components — even inside abstractions — are all the
    // same clock. Placing them anywhere is legal and they tick together.
    let mut e = Editor::empty();
    e.place_component("clk", [0, 0]).unwrap();
    e.place_component("clk", [10, 0]).unwrap();
    let i = e.create_abstraction("ticker").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("clk", [0, 0]).unwrap();
    e.place_component("dff", [6, 0]).unwrap();
    e.connect(port("clk_1", "out"), port("dff_1", "clk"), &[]).unwrap();
    e.connect(port("dff_1", "out"), port("dff_1", "d"), &[]).unwrap();
    e.connect(port("dff_1", "out"), pin("OUTPUT:", "q"), &[]).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("ticker", [0, 8]).unwrap();

    use crate::compiler::compile;
    use crate::convert::project_to_circuit;
    use crate::expander::expand;
    use crate::simulator::simulate;
    let expanded = expand(&project_to_circuit(&e.project).unwrap()).unwrap();
    let compiled = compile(&expanded.def).unwrap();
    let results = simulate(&compiled);
    // Both top-level clocks and the one inside the abstraction agree.
    for (step, state) in results.iter().enumerate() {
        assert_eq!(state["clk_1"], state["clk_2"]);
        assert_eq!(state["clk_1"], step % 2 == 1);
    }
}

#[test]
fn connect_in_either_order_and_replace_driver() {
    let mut e = editor_with_two_nands();
    // output → input
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
        .unwrap();
    // input → output (reversed argument order must also work)
    e.connect(port("nand_2", "b"), port("nand_1", "out"), &[])
        .unwrap();
    assert_eq!(e.project.wire_nets.len(), 1, "fan-out merges into one net");
    assert_eq!(e.project.wire_nets[0].sinks.len(), 2);

    // A third component takes over nand_2.a: old connection replaced.
    e.place_component("clk", [0, 10]).unwrap();
    e.connect(port("clk_1", "out"), port("nand_2", "a"), &[])
        .unwrap();
    let driving_a: Vec<_> = e
        .project
        .wire_nets
        .iter()
        .filter(|n| n.sinks.contains(&port("nand_2", "a")))
        .collect();
    assert_eq!(driving_a.len(), 1);
    assert_eq!(driving_a[0].source, Some(port("clk_1", "out")));
}

#[test]
fn connect_rejects_same_kind() {
    let mut e = editor_with_two_nands();
    assert_eq!(
        e.connect(port("nand_1", "out"), port("nand_2", "out"), &[])
            .unwrap_err(),
        EditError::NotAnInputAndOutput
    );
    assert_eq!(
        e.connect(port("nand_1", "a"), port("nand_2", "b"), &[])
            .unwrap_err(),
        EditError::NotAnInputAndOutput
    );
    assert!(e.project.wire_nets.is_empty());
}

#[test]
fn connect_with_bends_creates_junction_chain() {
    let mut e = editor_with_two_nands();
    e.connect(
        port("nand_1", "out"),
        port("nand_2", "a"),
        &[[5.0, 5.0], [7.0, 5.0]],
    )
    .unwrap();
    let net = &e.project.wire_nets[0];
    assert_eq!(net.junctions.len(), 2);
    assert_eq!(net.segments.len(), 3);
    // Path is connected: port → j1 → j2 → port.
    assert!(matches!(net.segments[0].from, Endpoint::Port { .. }));
    assert!(matches!(net.segments[2].to, Endpoint::Port { .. }));
}

#[test]
fn bends_reverse_when_drawn_from_input() {
    let mut e = editor_with_two_nands();
    // Drawn starting at the input, bending down then right.
    e.connect(
        port("nand_2", "a"),
        port("nand_1", "out"),
        &[[8.0, 5.0], [4.0, 5.0]],
    )
    .unwrap();
    let net = &e.project.wire_nets[0];
    // First junction in the stored path must be the one nearest the
    // SOURCE (bends reversed): [4,5] before [8,5].
    assert_eq!(net.junctions[0].pos, [4.0, 5.0]);
}

#[test]
fn delete_component_keeps_wire_stub() {
    let mut e = editor_with_two_nands();
    e.place_component("clk", [0, 10]).unwrap();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 1.0]])
        .unwrap();
    e.connect(port("nand_1", "out"), port("nand_2", "b"), &[])
        .unwrap();
    e.delete_components(&["nand_2".into()]).unwrap();
    assert_eq!(e.project.components.len(), 2);
    // Wires are placed objects: the source→node stub survives the
    // deletion of the component it pointed at.
    assert_eq!(e.project.wire_nets.len(), 1);
    let net = &e.project.wire_nets[0];
    assert_eq!(net.segments.len(), 1);
    assert_eq!(net.junctions.len(), 1);
    assert_eq!(net.source, Some(port("nand_1", "out")));
    assert!(net.sinks.is_empty());
}

#[test]
fn delete_source_component_keeps_nothing_dangling() {
    let mut e = editor_with_two_nands();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
        .unwrap();
    e.delete_components(&["nand_1".into()]).unwrap();
    // Sink-only net without a source connects nothing visual anymore:
    // the segment referenced the deleted port, so it's gone; and with
    // no source the net is pure junk → collected.
    assert!(e.project.wire_nets.is_empty());
}

#[test]
fn disconnect_single_sink_keeps_other_branch() {
    let mut e = editor_with_two_nands();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 0.0]])
        .unwrap();
    e.connect(port("nand_1", "out"), port("nand_2", "b"), &[[5.0, 3.0]])
        .unwrap();
    e.disconnect_sink(port("nand_2", "a")).unwrap();
    let net = &e.project.wire_nets[0];
    assert_eq!(net.sinks, vec![port("nand_2", "b")]);
    // Both nodes survive — the a-branch keeps its node as a stub.
    assert_eq!(net.junctions.len(), 2);
    // Disconnecting the other sink leaves a sourced net of stubs.
    e.disconnect_sink(port("nand_2", "b")).unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);
    assert!(e.project.wire_nets[0].sinks.is_empty());
    assert_eq!(
        e.project.wire_nets[0].source,
        Some(port("nand_1", "out"))
    );
}

#[test]
fn move_rejects_overlap_but_allows_group_moves() {
    let mut e = editor_with_two_nands(); // at x=0 and x=10
    assert_eq!(
        e.move_components(&["nand_1".into()], [9, 0]).unwrap_err(),
        EditError::Overlap
    );
    // Moving both together over each other's old spots is fine.
    e.move_components(&["nand_1".into(), "nand_2".into()], [10, 0])
        .unwrap();
    assert_eq!(e.project.components[0].pos, [10.0, 0.0]);
    assert_eq!(e.project.components[1].pos, [20.0, 0.0]);
}

#[test]
fn zero_move_is_not_an_undo_step() {
    let mut e = editor_with_two_nands();
    let undo_before = e.undo_stack.len();
    e.move_components(&["nand_1".into()], [0, 0]).unwrap();
    assert_eq!(e.undo_stack.len(), undo_before);
}

#[test]
fn undo_redo_round_trip() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    e.place_component("dff", [10, 0]).unwrap();
    e.connect(port("nand_1", "out"), port("dff_1", "d"), &[])
        .unwrap();

    e.undo().unwrap(); // un-connect
    assert!(e.project.wire_nets.is_empty());
    e.undo().unwrap(); // un-place dff
    assert_eq!(e.project.components.len(), 1);
    e.redo().unwrap();
    e.redo().unwrap();
    assert_eq!(e.project.components.len(), 2);
    assert_eq!(e.project.wire_nets.len(), 1);
    assert_eq!(e.redo().unwrap_err(), EditError::NothingToRedo);

    // A new edit clears the redo tail.
    e.undo().unwrap();
    e.place_component("clk", [20, 0]).unwrap();
    assert_eq!(e.redo().unwrap_err(), EditError::NothingToRedo);
}

#[test]
fn failed_op_is_not_undoable_and_rolls_back() {
    let mut e = editor_with_two_nands();
    let snapshot = e.project.clone();
    let _ = e.connect(port("nand_1", "out"), port("nand_2", "out"), &[]);
    assert_eq!(
        serde_json::to_string(&e.project).unwrap(),
        serde_json::to_string(&snapshot).unwrap()
    );
    // Two undos available (the two placements), not three.
    e.undo().unwrap();
    e.undo().unwrap();
    assert_eq!(e.undo().unwrap_err(), EditError::NothingToUndo);
}

#[test]
fn toggle_init() {
    let mut e = Editor::empty();
    e.place_component("dff", [0, 0]).unwrap();
    assert!(e.toggle_dff_init("dff_1").unwrap());
    assert!(!e.toggle_dff_init("dff_1").unwrap());
    e.place_component("nand", [10, 0]).unwrap();
    assert!(matches!(
        e.toggle_dff_init("nand_1").unwrap_err(),
        EditError::NotADff(_)
    ));
}

#[test]
fn self_connection_on_dff_is_allowed() {
    let mut e = Editor::empty();
    e.place_component("dff", [0, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("dff_1", "d"), &[])
        .unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);
}

// ── Abstraction sheets ───────────────────────────────────────────────

fn project_with_not_abstraction() -> Editor {
    let mut e = Editor::empty();
    e.project.abstractions.push(ProjectAbstraction {
        bundle_pins: HashMap::new(),
        pin_positions: HashMap::new(),
        port_layout: HashMap::new(),
        name: "not".into(),
        inputs: vec!["x".into()],
        outputs: HashMap::from([("out".into(), "gate".into())]),
        width: 4.0,
        height: 2.0,
        components: vec![ProjectComponent {
            display: None,
            bits: None,
            id: "gate".into(),
            component_type: "nand".into(),
            init: None,
            pos: [2.0, 2.0],
        }],
        wire_nets: vec![],
    notes: vec![],
        images: vec![],
    });
    e
}

#[test]
fn no_recursive_abstractions() {
    let mut e = project_with_not_abstraction();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    assert!(matches!(
        e.place_component("not", [8, 0]).unwrap_err(),
        EditError::RecursiveAbstraction(_)
    ));
    // But placing it at top level is fine.
    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("not", [0, 0]).unwrap();
}

#[test]
fn undo_restores_sheet() {
    let mut e = project_with_not_abstraction();
    e.place_component("nand", [0, 10]).unwrap();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    e.place_component("nand", [8, 0]).unwrap();
    e.undo().unwrap();
    assert_eq!(e.sheet, SheetRef::Abstraction(0));
    e.undo().unwrap();
    assert_eq!(e.sheet, SheetRef::Top);
}

// ── Wire graph (nodes as first-class objects) ────────────────────────

fn node(net_id: &str, junction_id: &str) -> WirePoint {
    WirePoint::Junction {
        net_id: net_id.into(),
        junction_id: junction_id.into(),
    }
}

#[test]
fn add_wire_incrementally_and_branch_from_node() {
    let mut e = editor_with_two_nands();
    // Out → node: a committed stub.
    let a = e
        .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 1.0]))
        .unwrap();
    let n1 = a.new_junction.unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);
    assert_eq!(e.project.wire_nets[0].sinks.len(), 0);
    assert_eq!(e.project.wire_nets[0].source, Some(port("nand_1", "out")));

    // Node → input finishes the connection.
    e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "a")))
        .unwrap();
    assert_eq!(e.project.wire_nets[0].sinks, vec![port("nand_2", "a")]);

    // Branch from the SAME node to the other input.
    e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "b")))
        .unwrap();
    assert_eq!(e.project.wire_nets[0].sinks.len(), 2);
    assert_eq!(e.project.wire_nets[0].segments.len(), 3);

    // Duplicate wire between the same points is a no-op.
    assert_eq!(
        e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "b")))
            .unwrap_err(),
        EditError::NoopWire
    );
}

#[test]
fn two_outputs_cannot_drive_one_wire() {
    let mut e = editor_with_two_nands();
    let a = e
        .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 5.0]))
        .unwrap();
    let n = a.new_junction.unwrap();
    assert!(matches!(
        e.add_wire(node(&a.net_id, &n), WirePoint::Port(port("nand_2", "out")))
            .unwrap_err(),
        EditError::TwoSources(_)
    ));
    // Rolled back: still one segment, and nand_2.out is untouched.
    assert_eq!(e.project.wire_nets[0].segments.len(), 1);
}

#[test]
fn merging_two_stub_nets_via_wire() {
    let mut e = editor_with_two_nands();
    // A sourced stub and a sunk stub.
    let a = e
        .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 0.0]))
        .unwrap();
    let b = e
        .add_wire(WirePoint::Port(port("nand_2", "a")), WirePoint::New([7.0, 3.0]))
        .unwrap();
    assert_eq!(e.project.wire_nets.len(), 2);
    // Wire node-to-node: nets merge and the connection goes live.
    e.add_wire(
        node(&a.net_id, &a.new_junction.clone().unwrap()),
        node(&b.net_id, &b.new_junction.clone().unwrap()),
    )
    .unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);
    let net = &e.project.wire_nets[0];
    assert_eq!(net.source, Some(port("nand_1", "out")));
    assert_eq!(net.sinks, vec![port("nand_2", "a")]);
    assert_eq!(net.segments.len(), 3);
    // Both stubs carried a junction named "j_1": the collision rename must
    // only touch the absorbed net's point, never fold the new wire into a
    // self-loop on the renamed one.
    assert!(net.segments.iter().all(|s| s.from != s.to));
}

#[test]
fn tied_inputs_get_driven_when_source_arrives() {
    let mut e = editor_with_two_nands();
    // Input-to-input wire: legal, a sourceless net with two sinks.
    e.add_wire(
        WirePoint::Port(port("nand_2", "a")),
        WirePoint::Port(port("nand_2", "b")),
    )
    .unwrap();
    let net = &e.project.wire_nets[0];
    assert_eq!(net.source, None);
    assert_eq!(net.sinks.len(), 2);
    // Attaching an output drives both.
    e.add_wire(
        WirePoint::Port(port("nand_1", "out")),
        WirePoint::Port(port("nand_2", "a")),
    )
    .unwrap();
    // The replace rule detached a first... so instead wire to a node of
    // that net: rebuild the scenario through a node.
    let mut e = editor_with_two_nands();
    let a = e
        .add_wire(WirePoint::Port(port("nand_2", "a")), WirePoint::New([5.0, 1.0]))
        .unwrap();
    let n = a.new_junction.unwrap();
    e.add_wire(node(&a.net_id, &n), WirePoint::Port(port("nand_2", "b")))
        .unwrap();
    e.add_wire(WirePoint::Port(port("nand_1", "out")), node(&a.net_id, &n))
        .unwrap();
    let net = &e.project.wire_nets[0];
    assert_eq!(net.source, Some(port("nand_1", "out")));
    assert_eq!(net.sinks.len(), 2);
}

#[test]
fn deleting_a_node_splits_the_net() {
    let mut e = editor_with_two_nands();
    // out → n1 → n2 → a  (a chain with two nodes)
    let a = e
        .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([4.0, 4.0]))
        .unwrap();
    let n1 = a.new_junction.unwrap();
    let b = e
        .add_wire(node(&a.net_id, &n1), WirePoint::New([8.0, 4.0]))
        .unwrap();
    let n2 = b.new_junction.unwrap();
    e.add_wire(node(&b.net_id, &n2), WirePoint::Port(port("nand_2", "a")))
        .unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);

    // Branch a second island off n2, then delete n2: the pieces that
    // met there fall apart into separate nets.
    let c = e
        .add_wire(node(&b.net_id, &n2), WirePoint::New([8.0, 8.0]))
        .unwrap();
    let n3 = c.new_junction.unwrap();
    e.delete_junction(&c.net_id, &n2).unwrap();

    // Remaining: out→n1 (sourced stub), and n3 alone died with its only
    // segment... n3's segment touched n2 so it's gone; nand_2.a's segment
    // also touched n2 and is gone.
    assert_eq!(e.project.wire_nets.len(), 1);
    let net = &e.project.wire_nets[0];
    assert_eq!(net.source, Some(port("nand_1", "out")));
    assert!(net.sinks.is_empty());
    assert!(net.junctions.iter().any(|j| j.id == n1));
    assert!(!net.junctions.iter().any(|j| j.id == n3));

    // Undo restores the whole arrangement.
    e.undo().unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);
    assert_eq!(e.project.wire_nets[0].junctions.len(), 3);
    assert_eq!(e.project.wire_nets[0].sinks, vec![port("nand_2", "a")]);
}

#[test]
fn splitting_preserves_both_live_halves() {
    let mut e = editor_with_two_nands();
    e.place_component("clk", [0, 10]).unwrap();
    // clk.out → n1 → n2, n2 → nand_2.a, and n1 → nand_2.b:
    let a = e
        .add_wire(WirePoint::Port(port("clk_1", "out")), WirePoint::New([4.0, 8.0]))
        .unwrap();
    let n1 = a.new_junction.unwrap();
    let b = e
        .add_wire(node(&a.net_id, &n1), WirePoint::New([8.0, 8.0]))
        .unwrap();
    let n2 = b.new_junction.unwrap();
    e.add_wire(node(&b.net_id, &n2), WirePoint::Port(port("nand_2", "a")))
        .unwrap();
    e.add_wire(node(&a.net_id, &n1), WirePoint::Port(port("nand_2", "b")))
        .unwrap();

    // Deleting n1 cuts clk and the b-branch off; n2→a survives as its
    // own (sourceless) net.
    e.delete_junction(&a.net_id, &n1).unwrap();
    assert_eq!(e.project.wire_nets.len(), 1);
    let net = &e.project.wire_nets[0];
    assert_eq!(net.source, None);
    assert_eq!(net.sinks, vec![port("nand_2", "a")]);
    assert_eq!(net.junctions.len(), 1);
}

#[test]
fn move_junction_updates_position() {
    let mut e = editor_with_two_nands();
    let a = e
        .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([5.0, 1.0]))
        .unwrap();
    let n = a.new_junction.unwrap();
    let undo_before = e.undo_stack.len();
    e.move_junction(&a.net_id, &n, [5.0, 1.0]).unwrap(); // no-op
    assert_eq!(e.undo_stack.len(), undo_before);
    e.move_junction(&a.net_id, &n, [9.0, 4.5]).unwrap();
    assert_eq!(e.project.wire_nets[0].junctions[0].pos, [9.0, 4.5]);
    assert!(matches!(
        e.move_junction(&a.net_id, "ghost", [0.0, 0.0]).unwrap_err(),
        EditError::UnknownJunction(_)
    ));
}

#[test]
fn output_pin_wired_from_a_node() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("buf").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "b"), &[]).unwrap();

    // Drive a node from the gate, then wire the node into the pin.
    let a = e
        .add_wire(WirePoint::Port(port("nand_1", "out")), WirePoint::New([9.0, 1.0]))
        .unwrap();
    let n = a.new_junction.unwrap();
    e.add_wire(node(&a.net_id, &n), WirePoint::Port(pin("OUTPUT:", "q")))
        .unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("q"),
        Some(&"nand_1".to_string())
    );

    // A sourceless node CAN feed an output pin now — the connection is a
    // real net, and the pin simply derives as unwired until a source
    // arrives (at which point the map updates by itself).
    let b = e
        .add_wire(WirePoint::Port(port("nand_1", "a")), WirePoint::New([2.0, 6.0]))
        .unwrap();
    // (nand_1.a was replaced-off the INPUT:x net onto a fresh stub.)
    let m = b.new_junction.unwrap();
    e.add_wire(node(&b.net_id, &m), WirePoint::Port(pin("OUTPUT:", "q")))
        .unwrap();
    assert_eq!(
        e.project.abstractions[0].outputs.get("q"),
        Some(&String::new()),
        "rewired to a sourceless net: derives unwired"
    );
}

// ── Interface pins ───────────────────────────────────────────────────

fn pin(kind_prefix: &str, name: &str) -> PortRef {
    PortRef {
        component_id: format!("{kind_prefix}{name}"),
        port_name: name.into(),
    }
}

#[test]
fn create_abstraction_and_edit_interface() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("adder").unwrap();
    assert!(matches!(
        e.create_abstraction("adder").unwrap_err(),
        EditError::NameTaken(_)
    ));
    assert!(matches!(
        e.create_abstraction("nand").unwrap_err(),
        EditError::NameTaken(_)
    ));
    assert!(matches!(
        e.create_abstraction("has space").unwrap_err(),
        EditError::InvalidName(_)
    ));

    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "a", None).unwrap();
    e.add_interface_pin(PortKind::Input, "b", None).unwrap();
    e.add_interface_pin(PortKind::Output, "sum", None).unwrap();
    assert!(matches!(
        e.add_interface_pin(PortKind::Output, "a", None).unwrap_err(),
        EditError::NameTaken(_)
    ));
    let def = &e.project.abstractions[i];
    assert_eq!(def.inputs, vec!["a", "b"]);
    assert_eq!(def.outputs.get("sum"), Some(&String::new()));
}

#[test]
fn wire_input_pin_through_gate_to_output_pin() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("buf2").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.add_interface_pin(PortKind::Output, "out", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();

    // Input pin drives both nand inputs (drawn from pin or port side).
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
    e.connect(port("nand_1", "b"), pin("INPUT:", "x"), &[]).unwrap();
    let nets = &e.project.abstractions[i].wire_nets;
    assert_eq!(nets.len(), 1, "one net per source, even for pins");
    assert_eq!(nets[0].sinks.len(), 2);

    // Gate output feeds the output pin: recorded in the outputs map.
    e.connect(port("nand_1", "out"), pin("OUTPUT:", "out"), &[])
        .unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("out"),
        Some(&"nand_1".to_string())
    );

    // Direct input→output passthrough is allowed: the output pin
    // simply forwards the input. (It replaces the nand_1 wiring.)
    e.connect(pin("INPUT:", "x"), pin("OUTPUT:", "out"), &[])
        .unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("out"),
        Some(&"INPUT:x".to_string())
    );
    e.undo().unwrap(); // back to the nand-driven output

    // The finished abstraction actually works end to end.
    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("dff", [0, 0]).unwrap();
    e.toggle_dff_init("dff_1").unwrap();
    e.place_component("buf2", [8, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("buf2_1", "x"), &[])
        .unwrap();
    let session_result = {
        use crate::compiler::compile;
        use crate::convert::project_to_circuit;
        use crate::expander::expand;
        use crate::simulator::simulate;
        let def = project_to_circuit(&e.project).unwrap();
        let expanded = expand(&def).unwrap();
        let compiled = compile(&expanded.def).unwrap();
        let mut results = simulate(&compiled);
        for step in &mut results {
            for (alias, target) in &expanded.aliases {
                if let Some(&v) = step.get(target) {
                    step.insert(alias.clone(), v);
                }
            }
        }
        results
    };
    // buf2 is NOT(NOT... actually a single nand with both inputs = x,
    // so out = NAND(1,1) = 0.
    assert!(!session_result[0]["buf2_1.out"]);
}

#[test]
fn remove_input_pin_scrubs_wires() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();

    e.remove_interface_pin(PortKind::Input, "x").unwrap();
    let def = &e.project.abstractions[i];
    assert!(def.inputs.is_empty());
    assert!(def.wire_nets.is_empty(), "pin's net is gone");
    // Undo brings both back.
    e.undo().unwrap();
    let def = &e.project.abstractions[i];
    assert_eq!(def.inputs, vec!["x"]);
    assert_eq!(def.wire_nets.len(), 1);
}

#[test]
fn deleting_component_unwires_output_pin() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[])
        .unwrap();
    e.delete_components(&["nand_1".into()]).unwrap();
    assert_eq!(
        e.project.abstractions[i].outputs.get("q"),
        Some(&String::new())
    );
}

#[test]
fn normalize_rejects_undeclared_input_pins() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();

    // A hand-edited file with a corrupted stored source but an intact
    // segment graph heals: the source is re-derived from the segments.
    let mut project = e.project.clone();
    project.abstractions[i].wire_nets[0].source = Some(pin("INPUT:", "ghost"));
    let notes = normalize(&mut project);
    assert_eq!(
        project.abstractions[i].wire_nets[0].source,
        Some(pin("INPUT:", "x"))
    );
    assert!(!notes.is_empty());

    // But segments referencing an undeclared pin are truly dangling.
    let mut project = e.project.clone();
    for net in &mut project.abstractions[i].wire_nets {
        net.source = Some(pin("INPUT:", "ghost"));
        for seg in &mut net.segments {
            for ep in [&mut seg.from, &mut seg.to] {
                if matches!(ep, Endpoint::Port { component_id, .. }
                    if component_id.starts_with("INPUT:"))
                {
                    *ep = Endpoint::Port {
                        component_id: "INPUT:ghost".into(),
                        port_name: "ghost".into(),
                    };
                }
            }
        }
    }
    let notes = normalize(&mut project);
    assert!(project.abstractions[i].wire_nets.is_empty());
    assert!(!notes.is_empty());

    // And an outputs map pointing at nothing gets unwired with a note.
    let mut project = e.project.clone();
    project.abstractions[i]
        .outputs
        .insert("q".into(), "ghost.out".into());
    let notes = normalize(&mut project);
    assert_eq!(project.abstractions[i].outputs["q"], "");
    assert!(!notes.is_empty());
}

#[test]
fn const_component_drives_signals() {
    let mut e = Editor::empty();
    e.place_component("const", [0, 0]).unwrap();
    assert!(e.toggle_dff_init("const_1").unwrap());
    e.place_component("nand", [6, 0]).unwrap();
    e.connect(port("const_1", "out"), port("nand_1", "a"), &[])
        .unwrap();
    // nand.b stays unconnected: reads as 0 → NAND(1, 0) = 1.
    use crate::compiler::compile;
    use crate::convert::project_to_circuit;
    use crate::expander::expand;
    use crate::simulator::simulate;
    let expanded = expand(&project_to_circuit(&e.project).unwrap()).unwrap();
    let compiled = compile(&expanded.def).unwrap();
    let results = simulate(&compiled);
    assert!(results[0]["const_1"]);
    assert!(results[0]["nand_1"]);
}

// ── Notes ────────────────────────────────────────────────────────────

#[test]
fn notes_place_edit_move_delete() {
    let mut e = Editor::empty();
    let id = e.place_note([2.0, 3.0], "reset logic lives here").unwrap();
    assert_eq!(id, "note_1");
    e.move_items(&[], std::slice::from_ref(&id), &[], [4, 1]).unwrap();
    assert_eq!(e.project.notes[0].pos, [6.0, 4.0]);
    e.set_note_text(&id, "updated").unwrap();
    assert_eq!(e.project.notes[0].text, "updated");
    // Empty text deletes.
    e.set_note_text(&id, "  ").unwrap();
    assert!(e.project.notes.is_empty());
    e.undo().unwrap();
    assert_eq!(e.project.notes[0].text, "updated");
}

#[test]
fn notes_delete_together_with_components() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    let note = e.place_note([5.0, 5.0], "hello").unwrap();
    e.delete_items(&["nand_1".into()], &[note], &[]).unwrap();
    assert!(e.project.components.is_empty());
    assert!(e.project.notes.is_empty());
    // One undo restores both (single op).
    e.undo().unwrap();
    assert_eq!(e.project.components.len(), 1);
    assert_eq!(e.project.notes.len(), 1);
}

// ── Renames & abstraction deletion ───────────────────────────────────

fn abstraction_with_instance() -> Editor {
    let mut e = Editor::empty();
    let i = e.create_abstraction("buf").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "b"), &[]).unwrap();
    e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[])
        .unwrap();
    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("dff", [0, 0]).unwrap();
    e.place_component("buf", [8, 0]).unwrap();
    // IDs are unique per sheet, so this is nand_1 at top level.
    let top_nand = e.place_component("nand", [16, 0]).unwrap();
    e.connect(port("dff_1", "out"), port("buf_1", "x"), &[]).unwrap();
    e.connect(port("buf_1", "q"), port(&top_nand, "a"), &[]).unwrap();
    e
}

#[test]
fn rename_input_pin_rewrites_everything() {
    let mut e = abstraction_with_instance();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    e.rename_interface_pin(PortKind::Input, "x", "data").unwrap();

    let def = &e.project.abstractions[0];
    assert_eq!(def.inputs, vec!["data"]);
    // Inside: nets now reference INPUT:data.
    assert_eq!(
        def.wire_nets[0].source,
        Some(pin("INPUT:", "data"))
    );
    // Outside: the instance's wired port follows.
    assert!(e
        .project
        .wire_nets
        .iter()
        .any(|n| n.sinks.contains(&port("buf_1", "data"))));
    // The circuit still compiles and runs.
    use crate::convert::project_to_circuit;
    assert!(project_to_circuit(&e.project).is_ok());
    let mut p = e.project.clone();
    assert!(normalize(&mut p).is_empty(), "rename left inconsistencies");
}

#[test]
fn rename_output_pin_rewrites_instance_wires_and_dot_targets() {
    let mut e = abstraction_with_instance();
    // Nest buf inside a second abstraction that exports its output.
    let j = e.create_abstraction("wrapper").unwrap();
    e.open_sheet(SheetRef::Abstraction(j)).unwrap();
    e.add_interface_pin(PortKind::Output, "w", None).unwrap();
    e.place_component("buf", [4, 0]).unwrap();
    e.connect(port("buf_1", "q"), pin("OUTPUT:", "w"), &[]).unwrap();
    assert_eq!(
        e.project.abstractions[j].outputs.get("w"),
        Some(&"buf_1.q".to_string())
    );

    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    e.rename_interface_pin(PortKind::Output, "q", "result").unwrap();

    assert!(e.project.abstractions[0].outputs.contains_key("result"));
    // Top-level wire from the instance's output follows.
    assert!(e
        .project
        .wire_nets
        .iter()
        .any(|n| n.source == Some(port("buf_1", "result"))));
    // The wrapper's dot target follows too.
    assert_eq!(
        e.project.abstractions[j].outputs.get("w"),
        Some(&"buf_1.result".to_string())
    );
    let mut p = e.project.clone();
    assert!(normalize(&mut p).is_empty(), "rename left inconsistencies");
}

#[test]
fn rename_pin_rejects_collisions() {
    let mut e = abstraction_with_instance();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    assert!(matches!(
        e.rename_interface_pin(PortKind::Input, "x", "q").unwrap_err(),
        EditError::NameTaken(_)
    ));
    assert!(matches!(
        e.rename_interface_pin(PortKind::Input, "x", "has space")
            .unwrap_err(),
        EditError::InvalidName(_)
    ));
}

#[test]
fn rename_abstraction_retypes_instances() {
    let mut e = abstraction_with_instance();
    e.rename_abstraction(0, "buffer").unwrap();
    assert_eq!(e.project.abstractions[0].name, "buffer");
    assert_eq!(e.project.components[1].component_type, "buffer");
    assert!(matches!(
        e.rename_abstraction(0, "nand").unwrap_err(),
        EditError::NameTaken(_)
    ));
    use crate::convert::project_to_circuit;
    assert!(project_to_circuit(&e.project).is_ok());
}

#[test]
fn rename_project_allows_free_text() {
    let mut e = Editor::empty();
    e.rename_project("My First CPU!").unwrap();
    assert_eq!(e.project.name, "My First CPU!");
    assert!(matches!(
        e.rename_project("   ").unwrap_err(),
        EditError::InvalidName(_)
    ));
}

#[test]
fn delete_abstraction_refused_while_used() {
    let mut e = abstraction_with_instance();
    assert!(matches!(
        e.delete_abstraction(0).unwrap_err(),
        EditError::AbstractionInUse(1)
    ));
    e.delete_components(&["buf_1".into()]).unwrap();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    e.delete_abstraction(0).unwrap();
    assert!(e.project.abstractions.is_empty());
    assert_eq!(e.sheet, SheetRef::Top, "deleting the open sheet goes home");
    // Undo brings it back (and the sheet restore comes from the stack).
    e.undo().unwrap();
    assert_eq!(e.project.abstractions.len(), 1);
}

#[test]
fn pins_have_stored_positions_and_order_ports() {
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "a", None).unwrap();
    e.add_interface_pin(PortKind::Input, "b", None).unwrap();
    // Both pins got concrete homes at creation.
    assert!(e.project.abstractions[i].pin_positions.contains_key("a"));
    assert!(e.project.abstractions[i].pin_positions.contains_key("b"));

    // Move "b" above "a": the instance's ports reorder to match.
    let a_y = e.project.abstractions[i].pin_positions["a"][1];
    e.set_pin_position("b", [-4.0, a_y - 3.0]).unwrap();
    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("thing", [0, 0]).unwrap();
    let comp = &e.project.components[0];
    let ports = layout::component_ports(&e.project.abstractions, comp);
    let inputs: Vec<&str> = ports
        .iter()
        .filter(|p| p.info.kind == PortKind::Input)
        .map(|p| p.info.name.as_str())
        .collect();
    assert_eq!(inputs, vec!["b", "a"], "port order mirrors inside layout");

    // Rename keeps the position; removal drops it.
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.rename_interface_pin(PortKind::Input, "b", "top").unwrap();
    assert!(e.project.abstractions[i].pin_positions.contains_key("top"));
    e.remove_interface_pin(PortKind::Input, "top").unwrap();
    assert!(!e.project.abstractions[i].pin_positions.contains_key("top"));
}

// ── Clipboard ────────────────────────────────────────────────────────

#[test]
fn fragment_paste_preserves_internal_wiring() {
    let mut e = editor_with_two_nands();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[5.0, 1.0]])
        .unwrap();
    e.place_note([0.0, 5.0], "keep me").unwrap();

    let frag = e.fragment(&["nand_1".into(), "nand_2".into()], &["note_1".into()], &[]);
    let (new_comps, new_notes, _) = e.paste_fragment(&frag, [0, 8], true).unwrap();
    assert_eq!(new_comps, vec!["nand_3", "nand_4"]);
    assert_eq!(new_notes, vec!["note_2"]);

    // The copies carry their own wire, junction included, and the
    // original wiring is untouched.
    assert_eq!(e.project.wire_nets.len(), 2);
    let copied_net = e
        .project
        .wire_nets
        .iter()
        .find(|n| n.source == Some(port("nand_3", "out")))
        .expect("copied net rewired to fresh ids");
    assert_eq!(copied_net.sinks, vec![port("nand_4", "a")]);
    assert_eq!(copied_net.junctions.len(), 1);
    assert_eq!(copied_net.junctions[0].pos, [5.0, 9.0]);

    // One undo removes the whole paste.
    e.undo().unwrap();
    assert_eq!(e.project.components.len(), 2);
    assert_eq!(e.project.wire_nets.len(), 1);
    assert_eq!(e.project.notes.len(), 1);
}

#[test]
fn fragment_excludes_wiring_to_uncopied_components() {
    let mut e = editor_with_two_nands();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
        .unwrap();
    // Copy only one endpoint: the wire between them must not come.
    let frag = e.fragment(&["nand_1".into()], &[], &[]);
    assert!(frag.wire_nets.is_empty());
    let (ids, _, _) = e.paste_fragment(&frag, [0, 8], true).unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(e.project.wire_nets.len(), 1, "no extra nets appeared");
}

#[test]
fn fragment_excludes_free_wiring_it_is_not_connected_to() {
    let mut e = editor_with_two_nands();
    // A wire routed through two nodes has a junction-junction segment; it
    // belongs to nand_1's net, not to whatever else gets copied later.
    e.connect(
        port("nand_1", "out"),
        port("nand_2", "a"),
        &[[6.0, 8.0], [7.0, 8.0]],
    )
    .unwrap();
    e.place_component("dff", [12, 12]).unwrap();
    let frag = e.fragment(&["dff_1".into()], &[], &[]);
    assert!(
        frag.wire_nets.is_empty(),
        "an unrelated component copy must not stow away other nets' wiring"
    );
}

#[test]
fn paste_searches_for_free_space_and_rejects_recursion() {
    let mut e = editor_with_two_nands();
    let frag = e.fragment(&["nand_1".into()], &[], &[]);
    // Zero offset collides with the original; search finds a spot.
    let (ids, _, _) = e.paste_fragment(&frag, [0, 0], true).unwrap();
    assert_eq!(ids.len(), 1);
    // Without search, a collision is refused outright.
    assert_eq!(
        e.paste_fragment(&frag, [0, 0], false).unwrap_err(),
        EditError::Overlap
    );

    // Recursion guard: an instance of X can't be pasted inside X.
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.place_component("thing", [0, 0]).unwrap();
    let frag = e.fragment(&["thing_1".into()], &[], &[]);
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    assert!(matches!(
        e.paste_fragment(&frag, [0, 0], true).unwrap_err(),
        EditError::RecursiveAbstraction(_)
    ));
}

#[test]
fn first_port_placement_freezes_the_rest_and_new_pins_find_slots() {
    use crate::project::PortSide;
    let mut e = Editor::empty();
    let i = e.create_abstraction("box").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    for name in ["a", "b", "c"] {
        e.add_interface_pin(PortKind::Input, name, None).unwrap();
    }
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    assert!(e.project.abstractions[i].port_layout.is_empty(), "auto until touched");

    // Positions before the first manual placement...
    let synth = ProjectComponent {
        display: None,
        bits: None,
        id: String::new(),
        component_type: "box".into(),
        init: None,
        pos: [0.0, 0.0],
    };
    let before: Vec<_> = layout::component_ports(&e.project.abstractions, &synth)
        .into_iter()
        .filter(|p| p.info.name != "a")
        .map(|p| (p.info.name.clone(), p.pos))
        .collect();

    // ...moving "a" to the top edge must not move anyone else.
    e.set_port_layout(i, "a", PortSide::Top, 0.5).unwrap();
    let after: Vec<_> = layout::component_ports(&e.project.abstractions, &synth)
        .into_iter()
        .filter(|p| p.info.name != "a")
        .map(|p| (p.info.name.clone(), p.pos))
        .collect();
    assert_eq!(before, after, "freezing must not shuffle other ports");
    assert_eq!(e.project.abstractions[i].port_layout.len(), 4);

    // A new pin lands in a free slot instead of re-flowing the layout.
    e.add_interface_pin(PortKind::Input, "d", None).unwrap();
    let spots = &e.project.abstractions[i].port_layout;
    assert!(spots.contains_key("d"));
    let occupied: Vec<i64> = spots
        .values()
        .filter(|s| s.side == PortSide::Left)
        .map(|s| (s.t * e.project.abstractions[i].height - 0.5).round() as i64)
        .collect();
    let mut dedup = occupied.clone();
    dedup.sort_unstable();
    dedup.dedup();
    assert_eq!(dedup.len(), occupied.len(), "no two left ports share a slot");
}

// ── Review regressions ───────────────────────────────────────────────

#[test]
fn expander_handles_right_to_left_wiring_inside_abstractions() {
    // H1: a sub-instance may reference one defined after it.
    let mut e = Editor::empty();
    let n = e.create_abstraction("inv").unwrap();
    e.open_sheet(SheetRef::Abstraction(n)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "b"), &[]).unwrap();
    e.connect(port("nand_1", "out"), pin("OUTPUT:", "q"), &[]).unwrap();

    let b = e.create_abstraction("buf2").unwrap();
    e.open_sheet(SheetRef::Abstraction(b)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    // Downstream instance placed FIRST, upstream second, wired
    // right-to-left: expansion must not depend on placement order.
    e.place_component("inv", [8, 0]).unwrap();
    e.place_component("inv", [0, 0]).unwrap();
    e.connect(port("inv_2", "q"), port("inv_1", "x"), &[]).unwrap();
    e.connect(port("inv_1", "q"), pin("OUTPUT:", "q"), &[]).unwrap();

    e.open_sheet(SheetRef::Top).unwrap();
    e.place_component("buf2", [0, 0]).unwrap();

    use crate::compiler::compile;
    use crate::convert::project_to_circuit;
    use crate::expander::expand;
    use crate::simulator::simulate;
    let expanded = expand(&project_to_circuit(&e.project).unwrap())
        .expect("wiring order must not matter");
    let compiled = compile(&expanded.def).unwrap();
    let results = simulate(&compiled);
    // inv_2 input unconnected → 0; NOT(0)=1; NOT(1)=0.
    let flat = expanded.aliases.get("buf2_1.q").unwrap();
    assert!(!results[0][flat]);
}

#[test]
fn removing_a_pin_scrubs_instance_wiring_everywhere() {
    // H2: without the scrub, every later op on the top sheet fails.
    let mut e = abstraction_with_instance();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    e.remove_interface_pin(PortKind::Input, "x").unwrap();
    e.open_sheet(SheetRef::Top).unwrap();
    // No net may still reference the vanished port...
    for net in &e.project.wire_nets {
        assert!(!net.sinks.contains(&port("buf_1", "x")));
    }
    // ...and wiring on the top sheet still works.
    e.place_component("nand", [0, 10]).unwrap();
    let new_nand = e.project.components.last().unwrap().id.clone();
    e.connect(port("dff_1", "out"), port(&new_nand, "a"), &[])
        .expect("sheet must not be poisoned by the removed pin");
    // And the design still expands.
    use crate::convert::project_to_circuit;
    use crate::expander::expand;
    assert!(expand(&project_to_circuit(&e.project).unwrap()).is_ok());

    // Output pins scrub too, including dot targets elsewhere.
    let mut e = abstraction_with_instance();
    e.open_sheet(SheetRef::Abstraction(0)).unwrap();
    e.remove_interface_pin(PortKind::Output, "q").unwrap();
    for net in &e.project.wire_nets {
        assert_ne!(net.source, Some(port("buf_1", "q")));
    }
}

#[test]
fn cyclic_abstractions_in_json_do_not_crash() {
    // H3: a → b → a cycle from a hand-edited file.
    let mut e = Editor::empty();
    e.create_abstraction("a").unwrap();
    e.create_abstraction("b").unwrap();
    e.create_abstraction("c").unwrap();
    e.project.abstractions[0].components.push(ProjectComponent {
        display: None,
        bits: None,
        id: "x".into(),
        component_type: "b".into(),
        init: None,
        pos: [0.0, 0.0],
    });
    e.project.abstractions[1].components.push(ProjectComponent {
        display: None,
        bits: None,
        id: "y".into(),
        component_type: "a".into(),
        init: None,
        pos: [0.0, 0.0],
    });
    let mut p = e.project.clone();
    normalize(&mut p); // must terminate
    e.open_sheet(SheetRef::Abstraction(2)).unwrap();
    // Placing into the cycle-adjacent sheet must not blow the stack.
    e.place_component("a", [0, 0]).unwrap();
    assert!(matches!(
        e.place_component("c", [10, 0]).unwrap_err(),
        EditError::RecursiveAbstraction(_)
    ));
}

#[test]
fn copies_never_stay_connected_to_unselected_things() {
    // Wires to interface pins are outside the selection, so they're
    // cut at copy time — same-sheet paste included.
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Input, "x", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    e.connect(pin("INPUT:", "x"), port("nand_1", "a"), &[]).unwrap();
    let frag = e.fragment(&["nand_1".into()], &[], &[]);
    assert!(
        frag.wire_nets.is_empty(),
        "the pin is not selected, so its wire must not travel"
    );

    // Same-sheet paste: the copy is NOT wired to the pin.
    let (ids, _, _) = e.paste_fragment(&frag, [0, 4], true).unwrap();
    assert_eq!(ids.len(), 1);
    assert_eq!(e.project.abstractions[i].wire_nets.len(), 1);
    let net = &e.project.abstractions[i].wire_nets[0];
    assert_eq!(net.sinks, vec![port("nand_1", "a")], "original only");

    // Cross-sheet paste works too.
    e.open_sheet(SheetRef::Top).unwrap();
    let (ids, _, _) = e.paste_fragment(&frag, [0, 0], true).unwrap();
    assert_eq!(ids.len(), 1);
    assert!(e.project.wire_nets.is_empty());
}

#[test]
fn normalize_canonicalizes_output_targets() {
    // M3: hand-written "gate.out" on a primitive, and bare instance.
    let mut e = Editor::empty();
    let i = e.create_abstraction("thing").unwrap();
    e.open_sheet(SheetRef::Abstraction(i)).unwrap();
    e.add_interface_pin(PortKind::Output, "q", None).unwrap();
    e.place_component("nand", [4, 0]).unwrap();
    let mut p = e.project.clone();
    p.abstractions[i].outputs.insert("q".into(), "nand_1.out".into());
    let notes = normalize(&mut p);
    // The map is derived from the synthesized net now, so hand-written
    // variants canonicalize silently — no repair note needed.
    assert_eq!(p.abstractions[i].outputs["q"], "nand_1");
    let _ = notes;
}

// ── Normalization ────────────────────────────────────────────────────

#[test]
fn normalize_synthesizes_segments_for_bare_nets() {
    let mut e = editor_with_two_nands();
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[])
        .unwrap();
    let mut project = e.project.clone();
    // Strip the visual half, as a hand-written file might.
    project.wire_nets[0].segments.clear();
    let notes = normalize(&mut project);
    assert_eq!(project.wire_nets[0].segments.len(), 1);
    assert!(notes.is_empty(), "synthesis is not a repair note: {notes:?}");
}

#[test]
fn normalize_merges_same_source_nets_and_dedups_drivers() {
    let mut project = editor_with_two_nands().project;
    let seg_less = |id: &str, sink: PortRef| WireNet {
        bundle: false,
        id: id.into(),
        source: Some(port("nand_1", "out")),
        sinks: vec![sink],
        junctions: vec![],
        segments: vec![],
    };
    project.wire_nets = vec![
        seg_less("w1", port("nand_2", "a")),
        seg_less("w2", port("nand_2", "b")),
        seg_less("w3", port("nand_2", "a")), // duplicate driver for a
    ];
    let notes = normalize(&mut project);
    assert_eq!(project.wire_nets.len(), 1);
    assert_eq!(project.wire_nets[0].sinks.len(), 2);
    assert!(!notes.is_empty());
}

#[test]
fn normalize_repairs_dangling_references() {
    let mut project = editor_with_two_nands().project;
    project.wire_nets = vec![WireNet {
        bundle: false,
        id: "w1".into(),
        source: Some(port("ghost", "out")),
        sinks: vec![port("nand_2", "a"), port("nand_2", "bogus_port")],
        junctions: vec![],
        segments: vec![],
    }];
    let notes = normalize(&mut project);
    // Source and bogus sink dropped; sourceless net keeps its one valid
    // sink but connects nothing → collected.
    assert!(project.wire_nets.is_empty());
    assert!(notes.len() >= 2);
}

#[test]
fn normalize_accepts_example_files() {
    for path in ["examples/toggle.json", "examples/counter.json"] {
        let text = std::fs::read_to_string(path).unwrap();
        let mut project: Project = serde_json::from_str(&text).unwrap();
        normalize(&mut project);
        // Every sink must have a visual path after normalization.
        for net in &project.wire_nets {
            for sink in &net.sinks {
                assert!(
                    net.segments.iter().any(|seg| [&seg.from, &seg.to]
                        .iter()
                        .any(|e| matches!(e, Endpoint::Port { component_id, port_name }
                            if component_id == &sink.component_id
                                && port_name == &sink.port_name))),
                    "{path}: sink {sink:?} has no segment"
                );
            }
        }
    }
}


#[test]
fn replacing_a_wired_input_leaves_no_haunted_stub() {
    let port = |c: &str, p: &str| PortRef {
        component_id: c.into(),
        port_name: p.into(),
    };
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    e.place_component("nand", [8, 0]).unwrap();
    // nand_1.out → node → nand_2.a
    e.connect(port("nand_1", "out"), port("nand_2", "a"), &[[6.0, 1.0]]).unwrap();

    // Drawing from the already-wired input replaces its old feed. The
    // old feed's tail must become its OWN net again — not a haunted
    // island still claiming nand_1 as the driver of the new stub.
    let attach = e
        .add_wire(
            WirePoint::Port(port("nand_2", "a")),
            WirePoint::New([10.0, 4.0]),
        )
        .unwrap();
    let stub = e
        .project
        .wire_nets
        .iter()
        .find(|n| n.id == attach.net_id)
        .unwrap();
    assert!(
        stub.source.is_none(),
        "the fresh stub has no driver: {:?}",
        stub.source
    );
    assert_eq!(stub.sinks, vec![port("nand_2", "a")]);
    // The old feed survives as a separate stub with its true source.
    assert!(e
        .project
        .wire_nets
        .iter()
        .any(|n| n.id != attach.net_id && n.source == Some(port("nand_1", "out"))));

    // And a new driver connects to the stub without a phantom conflict.
    e.place_component("nand", [0, 6]).unwrap();
    e.add_wire(
        WirePoint::Port(port("nand_3", "out")),
        WirePoint::Junction {
            net_id: attach.net_id.clone(),
            junction_id: attach.new_junction.clone().unwrap(),
        },
    )
    .unwrap();
    let net = e
        .project
        .wire_nets
        .iter()
        .find(|n| n.sinks.contains(&port("nand_2", "a")))
        .unwrap();
    assert_eq!(net.source, Some(port("nand_3", "out")));
}
