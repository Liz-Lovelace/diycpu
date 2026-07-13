
use diycpu::editor::SheetRef;
use diycpu::layout::PortKind;


use super::*;

use super::tests::*;

#[test]
fn note_lifecycle_through_gestures() {
    let mut d = Driver::new();
    // Place via palette + prompt.
    d.app.palette_clicked("note");
    d.click([4.0, 4.0]);
    assert!(d.app.prompt.is_some());
    d.type_text("hello world");
    d.app.handle_key(KeyInput::Space);
    d.type_text("note");
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.notes.len(), 1);
    assert_eq!(d.app.editor.project.notes[0].text, "hello world note");

    // Drag it somewhere else.
    let bounds = diycpu::layout::note_bounds(&d.app.editor.project.notes[0]);
    let center = [bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0];
    d.drag(center, [center[0] + 5.0, center[1] + 3.0]);
    let moved = &d.app.editor.project.notes[0];
    assert_eq!(moved.pos[0], bounds.x + 5.0);

    // Double-click opens the editor prefilled; emptying removes it.
    let bounds = diycpu::layout::note_bounds(&d.app.editor.project.notes[0]);
    d.move_to([bounds.x + bounds.w / 2.0, bounds.y + bounds.h / 2.0]);
    d.app.recompute_hover();
    d.app.double_clicked();
    assert!(d.app.prompt.is_some());
    let len = d.app.prompt.as_ref().unwrap().value.chars().count();
    for _ in 0..len {
        d.app.handle_key(KeyInput::Backspace);
    }
    d.app.handle_key(KeyInput::Enter);
    assert!(d.app.editor.project.notes.is_empty());
}

#[test]
fn rename_pin_via_double_click() {
    let mut d = Driver::new();
    d.app.action_new_abstraction();
    d.type_text("thing");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Input);
    d.type_text("x");
    d.app.handle_key(KeyInput::Enter);

    // Pin rails sit left of the content — pan them into the canvas.
    d.app.camera.offset = [700.0, 300.0];

    // Double-click the pin's name plate.
    let abstractions = &d.app.editor.project.abstractions;
    let pin = diycpu::layout::interface_pins(abstractions, &abstractions[0])
        .into_iter()
        .find(|p| p.name == "x")
        .unwrap();
    let plate = pin.plate;
    d.move_to([plate.x + plate.w / 2.0, plate.y + plate.h / 2.0]);
    d.app.recompute_hover();
    d.app.double_clicked();
    assert!(d.app.prompt.is_some());
    assert_eq!(d.app.prompt.as_ref().unwrap().value, "x");
    d.type_text("_in"); // appended to the prefill
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.abstractions[0].inputs, vec!["x_in"]);
}

#[test]
fn rename_project_and_abstraction_via_title() {
    let mut d = Driver::new();
    d.app.action_rename_title();
    assert!(d.app.prompt.is_some());
    let len = d.app.prompt.as_ref().unwrap().value.chars().count();
    for _ in 0..len {
        d.app.handle_key(KeyInput::Backspace);
    }
    d.type_text("my");
    d.app.handle_key(KeyInput::Space);
    d.type_text("cpu");
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.name, "my cpu");

    d.app.action_new_abstraction();
    d.type_text("adder");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_rename_title(); // now renames the open abstraction
    let len = d.app.prompt.as_ref().unwrap().value.chars().count();
    for _ in 0..len {
        d.app.handle_key(KeyInput::Backspace);
    }
    d.type_text("adder2");
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.abstractions[0].name, "adder2");

    // And delete it from its own sheet.
    d.app.action_delete_abstraction();
    assert!(d.app.editor.project.abstractions.is_empty());
    assert_eq!(d.app.editor.sheet, SheetRef::Top);
}
