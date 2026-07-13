
use diycpu::editor::SheetRef;


use super::*;

use super::tests::*;

fn build_project_with_instance(d: &mut Driver) {
    d.app.action_new_abstraction();
    d.type_text("box");
    d.app.handle_key(KeyInput::Enter);
    d.place("nand", [2.0, 1.0]);
    d.app.handle_key(KeyInput::Escape); // clear selection
    d.app.handle_key(KeyInput::Escape); // back to top
    assert_eq!(d.app.editor.sheet, SheetRef::Top);
    d.place("box", [5.0, 5.0]);
    d.app.handle_key(KeyInput::Escape);
}

/// Aim the camera so the given grid rect fills `fill`× the viewport,
/// centered.
fn aim(d: &mut Driver, rect: diycpu::layout::Rect, fill: f32) {
    let cr = d.app.canvas_rect();
    let v = cr.w.min(cr.h);
    let scale = fill * v / rect.w.max(rect.h);
    d.app.camera.zoom = scale / crate::camera::CELL;
    d.app.camera.offset = [
        cr.x + cr.w / 2.0 - (rect.x + rect.w / 2.0) * scale,
        cr.y + cr.h / 2.0 - (rect.y + rect.h / 2.0) * scale,
    ];
}

#[test]
fn zooming_in_descends_and_out_ascends_seamlessly() {
    let mut d = Driver::new();
    build_project_with_instance(&mut d);
    let abstractions = &d.app.editor.project.abstractions;
    let comp = &d.app.editor.project.components[0];
    let rect = diycpu::layout::footprint(abstractions, comp);

    // Big enough and centered: focus descends.
    aim(&mut d, rect, 1.5);
    let view_before = d.app.camera.view();
    let world_probe = [rect.x + 1.0, rect.y + 1.0]; // a point on the parent sheet
    let screen_before = view_before.to_screen(world_probe);

    d.app.update_focus();
    assert_eq!(d.app.editor.sheet, SheetRef::Abstraction(0));
    assert_eq!(d.app.path.len(), 1);

    // Seamless: the same world point still lands on the same pixel when
    // mapped through the rebased camera and the inverse of the map.
    let seg = d.app.path.last().unwrap().clone();
    let map = d.app.seg_map(&seg).unwrap();
    let parent_view = d.app.camera.view().through(&map.inverse());
    let screen_after = parent_view.to_screen(world_probe);
    assert!(
        (screen_before[0] - screen_after[0]).abs() < 0.5
            && (screen_before[1] - screen_after[1]).abs() < 0.5,
        "rebase moved the world: {screen_before:?} vs {screen_after:?}"
    );

    // Zoom the camera way out (in child coordinates): focus ascends.
    d.app.camera.zoom *= 0.2;
    d.app.update_focus();
    assert_eq!(d.app.editor.sheet, SheetRef::Top);
    assert!(d.app.path.is_empty());
}

#[test]
fn escape_ascends_one_level_and_editing_targets_focus() {
    let mut d = Driver::new();
    build_project_with_instance(&mut d);
    let rect = diycpu::layout::footprint(
        &d.app.editor.project.abstractions,
        &d.app.editor.project.components[0],
    );
    aim(&mut d, rect, 1.5);
    d.app.update_focus();
    assert_eq!(d.app.path.len(), 1);

    // Editing while focused edits the abstraction's sheet.
    d.app.camera.zoom = 1.0; // gestures at sane scale (no auto-ascend: no scroll)
    d.app.camera.offset = [400.0, 200.0];
    d.place("dff", [10.0, 1.0]);
    assert_eq!(d.app.editor.project.abstractions[0].components.len(), 2);
    assert!(!d.app.status.is_error, "{}", d.app.status.text);

    d.app.handle_key(KeyInput::Escape); // clear selection
    d.app.handle_key(KeyInput::Escape); // ascend
    assert!(d.app.path.is_empty());
    assert_eq!(d.app.editor.sheet, SheetRef::Top);
}

#[test]
fn breadcrumb_tracks_the_path() {
    let mut d = Driver::new();
    build_project_with_instance(&mut d);
    assert_eq!(d.app.breadcrumb(), vec!["untitled"]);
    let rect = diycpu::layout::footprint(
        &d.app.editor.project.abstractions,
        &d.app.editor.project.components[0],
    );
    aim(&mut d, rect, 1.5);
    d.app.update_focus();
    assert_eq!(d.app.breadcrumb(), vec!["untitled", "box"]);
    d.app.open_as_root(0);
    assert_eq!(d.app.breadcrumb(), vec!["untitled", "box"]);
    assert!(d.app.path.is_empty());
}

#[test]
fn undo_across_sheets_resets_the_view() {
    let mut d = Driver::new();
    build_project_with_instance(&mut d);
    let rect = diycpu::layout::footprint(
        &d.app.editor.project.abstractions,
        &d.app.editor.project.components[0],
    );
    aim(&mut d, rect, 1.5);
    d.app.update_focus();
    assert_eq!(d.app.path.len(), 1);
    // Undo jumps back to a top-sheet edit: view must follow.
    d.app.undo();
    assert_eq!(d.app.editor.sheet, d.app.focus_sheet());
}
