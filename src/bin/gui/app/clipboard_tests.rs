


use super::*;

use super::tests::*;

#[test]
fn copy_paste_via_keyboard() {
    let mut d = Driver::new();
    d.place("nand", [1.0, 1.0]);
    d.place("dff", [8.0, 1.0]);
    // Wire them, band-select everything, copy.
    d.drag(port_at(&d.app, "nand_1", "out"), port_at(&d.app, "dff_1", "d"));
    d.drag([-4.0, -3.0], [12.0, 4.0]);
    assert_eq!(d.app.selection.components.len(), 2);
    d.app.ctrl = true;
    d.app.handle_key(KeyInput::Char("c"));
    d.app.ctrl = false;
    assert!(d.app.clipboard.is_some());

    // Paste at a spot lower on the canvas.
    d.move_to([6.0, 12.0]);
    d.app.ctrl = true;
    d.app.handle_key(KeyInput::Char("v"));
    d.app.ctrl = false;
    assert_eq!(d.app.editor.project.components.len(), 4);
    assert_eq!(d.app.editor.project.wire_nets.len(), 2, "wiring came along");
    assert_eq!(d.app.selection.components.len(), 2, "copies are selected");
    assert!(!d.app.status.is_error, "{}", d.app.status.text);
}

#[test]
fn cut_removes_and_paste_restores() {
    let mut d = Driver::new();
    d.place("nand", [1.0, 1.0]);
    d.click(center_of(&d.app, "nand_1"));
    d.app.ctrl = true;
    d.app.handle_key(KeyInput::Char("x"));
    d.app.ctrl = false;
    assert!(d.app.editor.project.components.is_empty());
    d.move_to([5.0, 5.0]);
    d.app.ctrl = true;
    d.app.handle_key(KeyInput::Char("v"));
    d.app.ctrl = false;
    assert_eq!(d.app.editor.project.components.len(), 1);
}

#[test]
fn ctrl_drag_copies_and_ctrl_click_still_toggles() {
    let mut d = Driver::new();
    d.place("nand", [1.0, 1.0]);
    d.place("nand", [8.0, 1.0]);

    // Ctrl-click toggles selection (no drag).
    d.click(center_of(&d.app, "nand_1"));
    d.app.ctrl = true;
    d.click(center_of(&d.app, "nand_2"));
    assert_eq!(d.app.selection.components.len(), 2);
    d.click(center_of(&d.app, "nand_2"));
    assert_eq!(d.app.selection.components.len(), 1);
    d.app.ctrl = false;

    // Ctrl-drag copies the selection to the drop point.
    let original_pos = d.app.editor.project.components[0].pos;
    let from = center_of(&d.app, "nand_1");
    d.app.ctrl = true;
    d.drag(from, [from[0], from[1] + 6.0]);
    d.app.ctrl = false;
    assert_eq!(d.app.editor.project.components.len(), 3);
    let copy_id = &d.app.selection.components[0];
    let copy = d
        .app
        .editor
        .project
        .components
        .iter()
        .find(|c| &c.id == copy_id)
        .unwrap();
    assert_eq!(
        copy.pos,
        [original_pos[0], original_pos[1] + 6.0],
        "copy landed at the drop point"
    );
    // Original still where it was.
    assert_eq!(d.app.editor.project.components[0].pos, original_pos);
}
