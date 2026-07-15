
use diycpu::layout::PortKind;


use super::*;

use super::tests::*;

#[test]
fn bundle_wire_draws_between_bundle_ports_only() {
    let mut d = Driver::new();
    d.place("bundle", [2.0, 5.0]);
    d.place("unbundle", [8.0, 5.0]);
    d.place("nand", [14.0, 2.0]);

    // Hovering a signal port while dragging a bundle wire reads as
    // incompatible before the editor even gets asked.
    d.move_to(port_at(&d.app, "bundle_1", "bundle"));
    d.app.left_pressed();
    d.move_to(port_at(&d.app, "nand_1", "a"));
    assert!(matches!(d.app.wire_target(), WireTarget::Incompatible));
    d.app.handle_key(KeyInput::Escape);

    // Bundle port to bundle port connects and flags the net.
    d.drag(
        port_at(&d.app, "bundle_1", "bundle"),
        port_at(&d.app, "unbundle_1", "bundle"),
    );
    let nets = &d.app.editor.project.wire_nets;
    assert_eq!(nets.len(), 1, "{}", d.app.status.text);
    assert!(nets[0].bundle);

    // And lanes still take ordinary wires.
    d.drag(
        port_at(&d.app, "unbundle_1", "0"),
        port_at(&d.app, "nand_1", "a"),
    );
    assert_eq!(d.app.editor.project.wire_nets.len(), 2);
    assert!(!d.app.editor.project.wire_nets[1].bundle);
}

#[test]
fn cog_on_a_bundler_prompts_for_lanes() {
    let mut d = Driver::new();
    d.place("bundle", [4.0, 2.0]);
    d.move_to(center_of(&d.app, "bundle_1"));
    let (rect, action) = d.app.cog().expect("bundlers have settings");
    assert_eq!(
        action,
        CogAction::BundleWidth {
            id: "bundle_1".into()
        }
    );
    d.app.mouse_moved([rect.x + rect.w / 2.0, rect.y + rect.h / 2.0]);
    d.app.left_pressed();
    assert!(d.app.prompt.is_some());
    d.app.prompt.as_mut().unwrap().value.clear();
    d.type_text("4");
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.components[0].bits, Some(4));
    // The body resized with the lane count.
    let r = diycpu::layout::footprint(
        &d.app.editor.project.abstractions,
        &d.app.editor.project.components[0],
    );
    assert_eq!(r.h, 4.0);
    // A bundle of one is refused, with the prompt kept for a retry.
    d.move_to(center_of(&d.app, "bundle_1"));
    let (rect, _) = d.app.cog().unwrap();
    d.app.mouse_moved([rect.x + rect.w / 2.0, rect.y + rect.h / 2.0]);
    d.app.left_pressed();
    d.app.prompt.as_mut().unwrap().value.clear();
    d.type_text("1");
    d.app.handle_key(KeyInput::Enter);
    assert!(d.app.status.is_error);
    assert_eq!(d.app.editor.project.components[0].bits, Some(4));
    d.app.handle_key(KeyInput::Escape);
}

#[test]
fn bundle_pins_come_from_the_name_colon_width_syntax() {
    let mut d = Driver::new();
    d.app.action_new_abstraction();
    d.type_text("bus");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Input);
    d.type_text("data:8");
    d.app.handle_key(KeyInput::Enter);
    assert!(!d.app.status.is_error, "{}", d.app.status.text);
    let def = &d.app.editor.project.abstractions[0];
    assert_eq!(def.inputs, vec!["data".to_string()]);
    assert_eq!(def.bundle_pins.get("data"), Some(&8));

    // A bad width keeps the prompt for a retry.
    d.app.action_add_pin(PortKind::Output);
    d.type_text("q:1");
    d.app.handle_key(KeyInput::Enter);
    assert!(d.app.status.is_error);
    assert!(d.app.prompt.is_some());
    d.app.handle_key(KeyInput::Escape);
}


#[test]
fn display_cog_opens_settings_and_configures() {
    let mut d = Driver::new();
    d.place("light", [4.0, 4.0]);
    d.move_to(center_of(&d.app, "light_1"));
    let (rect, action) = d.app.cog().expect("displays have settings");
    assert_eq!(
        action,
        CogAction::DisplaySettings {
            id: "light_1".into()
        }
    );
    d.app.mouse_moved([rect.x + rect.w / 2.0, rect.y + rect.h / 2.0]);
    d.app.left_pressed();
    assert!(d.app.display_dialog.is_some());

    // Round shape via the variant button.
    let geom = d.app.display_dialog_geometry().unwrap();
    let round = geom.variants.iter().find(|(l, _, _)| l == "Round").unwrap().1;
    d.app.cursor = [round.x + 4.0, round.y + 4.0];
    d.app.display_dialog_pressed();
    assert_eq!(
        d.app.editor.project.components[0]
            .display
            .as_ref()
            .and_then(|c| c.round),
        Some(true)
    );

    // A color from the swatch row.
    let geom = d.app.display_dialog_geometry().unwrap();
    let swatch = geom.swatches[3].clone();
    d.app.cursor = [swatch.1.x + 4.0, swatch.1.y + 4.0];
    d.app.display_dialog_pressed();
    assert_eq!(
        d.app.editor.project.components[0]
            .display
            .as_ref()
            .and_then(|c| c.color.clone()),
        Some(swatch.0)
    );

    // Width slider dragged to the far end.
    let geom = d.app.display_dialog_geometry().unwrap();
    let track = geom.sliders[0].track;
    d.app.cursor = [track.x + 4.0, track.y + track.h / 2.0];
    d.app.display_dialog_pressed();
    d.app.cursor = [track.x + track.w, track.y + track.h / 2.0];
    d.app.display_dialog_moved();
    d.app.display_dialog_released();
    assert_eq!(
        diycpu::layout::light_size(&d.app.editor.project.components[0])[0],
        DISPLAY_SIZE_MAX
    );

    // Esc closes; modality held while open.
    d.app.handle_key(KeyInput::Escape);
    assert!(d.app.display_dialog.is_none());
}

#[test]
fn seg_type_switch_via_dialog() {
    let mut d = Driver::new();
    d.place("seg", [6.0, 4.0]);
    d.app.action_open_display_dialog("seg_1");
    let geom = d.app.display_dialog_geometry().unwrap();
    let eight = geom
        .variants
        .iter()
        .find(|(l, _, _)| l.starts_with('8'))
        .unwrap()
        .1;
    d.app.cursor = [eight.x + 4.0, eight.y + 4.0];
    d.app.display_dialog_pressed();
    assert_eq!(d.app.editor.project.components[0].bits, Some(8));
    // The footprint follows the 8-segment aspect.
    let r = diycpu::layout::footprint(
        &d.app.editor.project.abstractions,
        &d.app.editor.project.components[0],
    );
    assert!((r.w / r.h - diycpu::layout::SEG8_ASPECT).abs() < 1e-3);
}
