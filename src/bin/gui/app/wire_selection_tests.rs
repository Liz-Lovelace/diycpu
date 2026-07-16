
use diycpu::layout::PortKind;


use super::*;

use super::tests::*;

fn wired_pair(d: &mut Driver) {
    d.place("nand", [1.0, 1.0]);
    d.place("nand", [9.0, 5.0]);
    d.drag(port_at(&d.app, "nand_1", "out"), port_at(&d.app, "nand_2", "a"));
    assert_eq!(d.app.editor.project.wire_nets.len(), 1);
    d.app.handle_key(KeyInput::Escape); // clear selection
}

fn segment_mid(d: &Driver) -> [f32; 2] {
    let a = port_at(&d.app, "nand_1", "out");
    let b = port_at(&d.app, "nand_2", "a");
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

#[test]
fn click_selects_a_segment_and_delete_removes_it() {
    let mut d = Driver::new();
    wired_pair(&mut d);
    d.click(segment_mid(&d));
    assert_eq!(d.app.selection.segments.len(), 1);
    // Move the cursor off the wire so hover-delete doesn't shortcut.
    d.move_to([0.0, 12.0]);
    d.app.handle_key(KeyInput::Delete);
    assert!(d.app.editor.project.wire_nets.is_empty());
}

#[test]
fn dragging_a_wire_tears_out_a_node_in_one_motion() {
    let mut d = Driver::new();
    wired_pair(&mut d);
    let mid = segment_mid(&d);
    d.drag(mid, [mid[0], mid[1] + 4.0]);
    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(net.junctions.len(), 1, "a node was born from the drag");
    let expected_y = ((mid[1] + 4.0) * 2.0).round() / 2.0; // drag end, half-snapped
    assert_eq!(net.junctions[0].pos[1], expected_y);
    assert_eq!(net.segments.len(), 2);
    // Still one connected net with the same logic.
    assert_eq!(net.sinks.len(), 1);
}

#[test]
fn box_select_includes_wires() {
    let mut d = Driver::new();
    wired_pair(&mut d);
    d.drag([-2.0, -1.0], [14.0, 9.0]);
    assert_eq!(d.app.selection.components.len(), 2);
    assert_eq!(d.app.selection.segments.len(), 1, "the wire came along");
    d.app.handle_key(KeyInput::Delete);
    assert!(d.app.editor.project.components.is_empty());
    assert!(d.app.editor.project.wire_nets.is_empty());
}

#[test]
fn output_pin_wire_selects_and_unwires() {
    let mut d = Driver::new();
    d.app.action_new_abstraction();
    d.type_text("box");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Output);
    d.type_text("q");
    d.app.handle_key(KeyInput::Enter);
    d.place("nand", [2.0, 1.0]);
    d.app.camera.offset = [300.0, 250.0];
    let abstractions = &d.app.editor.project.abstractions;
    let pin_pos = diycpu::layout::interface_pins(abstractions, &abstractions[0])
        .into_iter()
        .find(|p| p.name == "q")
        .unwrap()
        .pos;
    d.drag(port_at(&d.app, "nand_1", "out"), pin_pos);
    assert_eq!(
        d.app.editor.project.abstractions[0].outputs.get("q"),
        Some(&"nand_1".to_string())
    );

    // The pin wire is an ordinary segment: click it midway, delete it,
    // and the outputs map derives back to unwired all by itself.
    let from = port_at(&d.app, "nand_1", "out");
    let mid = [(from[0] + pin_pos[0]) / 2.0, (from[1] + pin_pos[1]) / 2.0];
    d.app.handle_key(KeyInput::Escape);
    d.click(mid);
    assert_eq!(d.app.selection.segments.len(), 1, "{:?}", d.app.hover);
    d.move_to([0.0, -6.0]);
    d.app.handle_key(KeyInput::Delete);
    assert_eq!(
        d.app.editor.project.abstractions[0].outputs.get("q"),
        Some(&String::new())
    );
}


#[test]
fn drawing_onto_a_wire_tees_in_as_one_step() {
    let mut d = Driver::new();
    d.place("dff", [3.0, 2.0]);
    d.place("nand", [12.0, 2.0]);
    d.place("nand", [12.0, 7.0]);
    let from = port_at(&d.app, "dff_1", "out");
    let to = port_at(&d.app, "nand_1", "a");
    d.drag(from, to);
    assert_eq!(d.app.editor.project.wire_nets.len(), 1);

    // Draw from the second gate's input and click MID-WIRE: that's a tee —
    // a node splits the segment and the wire joins it, one undo step.
    let mid = [(from[0] + to[0]) / 2.0, from[1]];
    d.click(port_at(&d.app, "nand_2", "a"));
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    d.click(mid);
    assert!(matches!(d.app.gesture, Gesture::Idle), "{}", d.app.status.text);
    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(d.app.editor.project.wire_nets.len(), 1);
    assert_eq!(net.junctions.len(), 1);
    assert_eq!(net.sinks.len(), 2);

    d.app.undo();
    let net = &d.app.editor.project.wire_nets[0];
    assert_eq!(net.sinks.len(), 1, "the whole tee was one step");
    assert!(net.junctions.is_empty());
}

#[test]
fn mid_wire_click_with_a_second_driver_refuses_and_stacks_nothing() {
    let mut d = Driver::new();
    d.place("dff", [3.0, 2.0]);
    d.place("nand", [12.0, 2.0]);
    d.place("clk", [3.0, 7.0]);
    let from = port_at(&d.app, "dff_1", "out");
    let to = port_at(&d.app, "nand_1", "a");
    d.drag(from, to);

    // The user's trap: trying to drive an already-driven net from the
    // clock. It must refuse — and it must NOT leave a node sitting on the
    // wire pretending to be connected.
    let mid = [(from[0] + to[0]) / 2.0, from[1]];
    d.click(port_at(&d.app, "clk_1", "out"));
    d.click(mid);
    assert!(d.app.status.is_error, "refused: {}", d.app.status.text);
    assert!(matches!(d.app.gesture, Gesture::DrawingWire { .. }));
    let net = &d.app.editor.project.wire_nets[0];
    assert!(net.junctions.is_empty(), "no phantom node on the wire");
    assert_eq!(d.app.editor.project.wire_nets.len(), 1, "no stub net either");
    d.app.handle_key(KeyInput::Escape);
}

#[test]
fn snapped_free_points_join_wiring_instead_of_stacking() {
    let mut d = Driver::new();
    d.place("dff", [3.0, 2.0]);
    d.place("nand", [12.0, 2.0]);
    d.place("nand", [3.0, 7.0]);
    let from = port_at(&d.app, "dff_1", "out");
    let to = port_at(&d.app, "nand_1", "a");
    d.drag(from, to);

    // Start a wire, then park the cursor over the FIRST gate's body right
    // where the wire passes underneath-ish: hover says Component, but the
    // snapped point lands on the wire — the target must be the wire, not
    // a free node stacked on top of it.
    d.click(port_at(&d.app, "nand_2", "a"));
    let over_wire = [(from[0] + to[0]) / 2.0, from[1] + 0.1];
    d.move_to(over_wire);
    match d.app.wire_target() {
        WireTarget::Segment { .. } | WireTarget::Node { .. } => {}
        other => panic!("expected a join target, got {other:?}"),
    }
    d.app.handle_key(KeyInput::Escape);
}
