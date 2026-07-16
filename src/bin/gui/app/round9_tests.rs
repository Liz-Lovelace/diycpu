
use diycpu::layout::PortKind;


use super::*;

use super::tests::*;

#[test]
fn band_grabs_partially_covered_wires() {
    let mut d = Driver::new();
    d.place("nand", [1.5, 1.0]); // body at (0,0)..(3,2)
    d.place("nand", [13.5, 1.0]); // body at (12,0)..(15,2)
    d.drag(port_at(&d.app, "nand_1", "out"), port_at(&d.app, "nand_2", "a"));
    assert_eq!(d.app.editor.project.wire_nets.len(), 1);

    // A band over just the MIDDLE of the wire — no endpoint inside.
    d.drag([6.0, -1.0], [8.0, 3.0]);
    assert_eq!(d.app.selection.segments.len(), 1, "part of a wire is enough");
    assert!(d.app.selection.components.is_empty());
}

#[test]
fn pins_wire_to_pins_and_refuse_lying_drops() {
    let mut d = Driver::new();
    d.app.action_new_abstraction();
    d.type_text("buf");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Input);
    d.type_text("x");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Output);
    d.type_text("q");
    d.app.handle_key(KeyInput::Enter);
    // Park both pins in open canvas (the defaults sit at the left rail,
    // where the Driver's framing puts them under the palette).
    d.app.editor.set_pin_position("x", [4.0, 6.0]).unwrap();
    d.app.editor.set_pin_position("q", [14.0, 6.0]).unwrap();

    let pin_geom = |app: &App, name: &str| {
        let abstractions = &app.editor.project.abstractions;
        let def = &abstractions[0];
        diycpu::layout::interface_pins(abstractions, def)
            .into_iter()
            .find(|p| p.name == name)
            .unwrap()
    };

    // Drag a wire from the input pin's circle to the output pin's
    // circle: a passthrough connection.
    let x = pin_geom(&d.app, "x").pos;
    let q = pin_geom(&d.app, "q").pos;
    d.drag(x, q);
    assert_eq!(
        d.app.editor.project.abstractions[0].outputs.get("q"),
        Some(&"INPUT:x".to_string()),
        "{}",
        d.app.status.text
    );

    // The pin-to-pin wire is a REAL net now: hovering its middle finds an
    // ordinary segment.
    d.move_to([(x[0] + q[0]) / 2.0, (x[1] + q[1]) / 2.0]);
    assert!(
        matches!(d.app.hover, diycpu::layout::Hit::Wire { .. }),
        "{:?}",
        d.app.hover
    );

    // Dropping the input pin ON the wire is refused: it would
    // look connected without being a connection.
    let x_pin = pin_geom(&d.app, "x");
    let plate_center = [
        x_pin.plate.x + x_pin.plate.w / 2.0,
        x_pin.plate.y + x_pin.plate.h / 2.0,
    ];
    let before = x_pin.pos;
    let mid = [(x[0] + q[0]) / 2.0, (x[1] + q[1]) / 2.0];
    d.drag(plate_center, mid);
    assert!(d.app.status.is_error, "drop refused: {}", d.app.status.text);
    assert_eq!(pin_geom(&d.app, "x").pos, before, "the pin stayed put");

    // A drop in the open works as always.
    d.drag(plate_center, [before[0], before[1] + 4.0]);
    assert_ne!(pin_geom(&d.app, "x").pos, before);
}

#[test]
fn pin_cannot_be_dropped_on_a_component() {
    let mut d = Driver::new();
    d.app.action_new_abstraction();
    d.type_text("box");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Input);
    d.type_text("x");
    d.app.handle_key(KeyInput::Enter);
    d.app.editor.set_pin_position("x", [4.0, 6.0]).unwrap();
    d.place("nand", [6.0, 1.0]);

    let abstractions = &d.app.editor.project.abstractions;
    let pin = diycpu::layout::interface_pins(abstractions, &abstractions[0])
        .into_iter()
        .find(|p| p.name == "x")
        .unwrap();
    let plate_center = [pin.plate.x + pin.plate.w / 2.0, pin.plate.y + pin.plate.h / 2.0];
    d.drag(plate_center, center_of(&d.app, "nand_1"));
    assert!(d.app.status.is_error);
    assert!(d.app.status.text.contains("component"), "{}", d.app.status.text);
}


#[test]
fn wiring_a_loop_is_refused_at_gesture_level() {
    let mut d = Driver::new();
    d.place("nand", [1.5, 1.0]);
    d.drag(port_at(&d.app, "nand_1", "out"), port_at(&d.app, "nand_1", "a"));
    assert!(d.app.status.is_error);
    assert!(
        d.app.status.text.contains("flip-flop"),
        "helpful message: {}",
        d.app.status.text
    );
    assert!(d.app.editor.project.wire_nets.is_empty(), "nothing was created");
}
