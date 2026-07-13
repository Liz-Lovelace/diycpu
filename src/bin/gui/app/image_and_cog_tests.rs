
use diycpu::editor::SheetRef;
use diycpu::layout::PortKind;


use super::*;

use super::tests::*;

fn place_test_image(d: &mut Driver, pos: [f32; 2], size: [f32; 2]) -> String {
    let id = d.app.editor.place_image(pos, "media/test.png", size).unwrap();
    d.app.recompute_hover();
    id
}

#[test]
fn image_selects_drags_and_deletes_like_anything_else() {
    let mut d = Driver::new();
    let id = place_test_image(&mut d, [2.0, 2.0], [6.0, 4.0]);

    // Click selects.
    d.click([5.0, 4.0]);
    assert_eq!(d.app.selection.images, vec![id.clone()]);

    // Drag moves (whole-cell deltas, like components).
    d.drag([5.0, 4.0], [10.0, 6.0]);
    assert_eq!(d.app.editor.project.images[0].pos, [7.0, 4.0]);

    // Box-select sweeps it up together with a component.
    d.place("nand", [16.0, 2.0]);
    d.drag([5.0, 2.0], [19.0, 9.0]);
    assert_eq!(d.app.selection.images.len(), 1);
    assert_eq!(d.app.selection.components.len(), 1);

    // Delete removes the whole mix in one undo step.
    d.app.handle_key(KeyInput::Delete);
    assert!(d.app.editor.project.images.is_empty());
    assert!(d.app.editor.project.components.is_empty());
    d.app.undo();
    assert_eq!(d.app.editor.project.images.len(), 1);
    assert_eq!(d.app.editor.project.components.len(), 1);
}

#[test]
fn clipboard_carries_images() {
    let mut d = Driver::new();
    let id = place_test_image(&mut d, [2.0, 2.0], [4.0, 3.0]);
    d.click([4.0, 3.5]);
    assert_eq!(d.app.selection.images, vec![id]);
    d.app.copy_selection();
    d.move_to([20.0, 10.0]);
    d.app.paste_clipboard();
    assert_eq!(d.app.editor.project.images.len(), 2);
    assert_eq!(d.app.selection.images.len(), 1);
    assert_eq!(d.app.editor.project.images[1].file, "media/test.png");
}

#[test]
fn ctrl_drag_copies_an_image() {
    let mut d = Driver::new();
    place_test_image(&mut d, [2.0, 2.0], [4.0, 3.0]);
    d.app.ctrl = true;
    d.drag([4.0, 3.5], [14.0, 3.5]);
    d.app.ctrl = false;
    assert_eq!(d.app.editor.project.images.len(), 2);
    assert_eq!(d.app.editor.project.images[0].pos, [2.0, 2.0]);
    assert_eq!(d.app.editor.project.images[1].pos, [12.0, 2.0]);
}

#[test]
fn cog_on_dff_toggles_start_value() {
    let mut d = Driver::new();
    d.place("dff", [4.0, 4.0]);
    let center = center_of(&d.app, "dff_1");
    d.move_to(center);
    let (rect, action) = d.app.cog().expect("dff shows a cog");
    assert_eq!(
        action,
        CogAction::ToggleInit {
            id: "dff_1".into()
        }
    );
    // Click inside the cog: the init flips, nothing gets selected.
    d.app.mouse_moved([rect.x + rect.w / 2.0, rect.y + rect.h / 2.0]);
    d.app.left_pressed();
    d.app.left_released();
    assert_eq!(d.app.editor.project.components[0].init, Some(true));
}

#[test]
fn cog_on_instance_opens_the_layout_dialog_from_any_sheet() {
    let mut d = Driver::new();
    d.app.action_new_abstraction();
    d.type_text("box");
    d.app.handle_key(KeyInput::Enter);
    d.app.action_add_pin(PortKind::Input);
    d.type_text("x");
    d.app.handle_key(KeyInput::Enter);
    d.place("nand", [2.0, 2.0]);
    d.app.action_back_to_top();
    d.place("box", [4.0, 4.0]);

    d.move_to(center_of(&d.app, "box_1"));
    let (rect, action) = d.app.cog().expect("instances show a cog");
    assert_eq!(action, CogAction::Layout { index: 0 });
    d.app.mouse_moved([rect.x + rect.w / 2.0, rect.y + rect.h / 2.0]);
    d.app.left_pressed();
    d.app.left_released();
    // The dialog opened for the right abstraction — from the TOP sheet.
    assert_eq!(d.app.layout_dialog.as_ref().map(|l| l.index), Some(0));
    assert_eq!(d.app.editor.sheet, SheetRef::Top);
}

#[test]
fn no_cog_on_gates_and_none_while_running() {
    let mut d = Driver::new();
    d.place("nand", [4.0, 4.0]);
    d.move_to(center_of(&d.app, "nand_1"));
    assert!(d.app.cog().is_none(), "a bare gate has no settings");

    d.place("dff", [10.0, 4.0]);
    d.move_to(center_of(&d.app, "dff_1"));
    assert!(d.app.cog().is_some());
    d.app.action_run_toggle();
    d.move_to(center_of(&d.app, "dff_1"));
    assert!(d.app.cog().is_none(), "no cog while running");
}

#[test]
fn cog_on_image_opens_settings_and_sliders_resize() {
    let mut d = Driver::new();
    let id = place_test_image(&mut d, [2.0, 2.0], [6.0, 4.0]);
    d.move_to([5.0, 4.0]);
    let (rect, action) = d.app.cog().expect("images show a cog");
    assert_eq!(action, CogAction::ImageSettings { id: id.clone() });
    d.app.mouse_moved([rect.x + rect.w / 2.0, rect.y + rect.h / 2.0]);
    d.app.left_pressed();
    assert!(d.app.image_dialog.is_some());

    // Drag the width slider to its far end.
    let geom = d.app.image_dialog_geometry().unwrap();
    let track = geom.sliders[0].track;
    d.app.cursor = [track.x + 4.0, track.y + track.h / 2.0];
    d.app.img_dialog_pressed();
    d.app.cursor = [track.x + track.w, track.y + track.h / 2.0];
    d.app.img_dialog_moved();
    d.app.img_dialog_released();
    assert_eq!(d.app.editor.project.images[0].size[0], IMG_SIZE_MAX);

    // Type the height instead: value button → prompt → commit.
    let geom = d.app.image_dialog_geometry().unwrap();
    let v = geom.sliders[1].value_rect;
    d.app.cursor = [v.x + 2.0, v.y + 2.0];
    d.app.img_dialog_pressed();
    assert!(d.app.prompt.is_some());
    d.app.prompt.as_mut().unwrap().value.clear();
    d.type_text("2.5");
    d.app.handle_key(KeyInput::Enter);
    assert_eq!(d.app.editor.project.images[0].size, [IMG_SIZE_MAX, 2.5]);

    // Esc closes; editing is unlocked again.
    d.app.handle_key(KeyInput::Escape);
    assert!(d.app.image_dialog.is_none());
}

#[test]
fn image_dialog_is_modal() {
    let mut d = Driver::new();
    let id = place_test_image(&mut d, [2.0, 2.0], [6.0, 4.0]);
    d.app.action_open_image_dialog(&id);
    let zoom = d.app.camera.zoom;
    d.app.scrolled(120.0);
    assert_eq!(d.app.camera.zoom, zoom, "wheel is swallowed");
    let geom = d.app.image_dialog_geometry().unwrap();
    // A click outside the panel closes it and does nothing else.
    d.app.cursor = [geom.panel.x - 40.0, geom.panel.y - 40.0];
    d.app.img_dialog_pressed();
    assert!(d.app.image_dialog.is_none());
}
