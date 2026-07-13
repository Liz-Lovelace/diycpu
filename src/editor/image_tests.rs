

use super::*;

#[test]
fn image_lifecycle_place_move_resize_delete_undo() {
    let mut e = Editor::empty();
    let id = e.place_image([2.0, 3.0], "media/cat.png", [6.0, 4.0]).unwrap();
    assert_eq!(e.project.images.len(), 1);
    assert_eq!(e.project.images[0].file, "media/cat.png");

    e.move_items(&[], &[], std::slice::from_ref(&id), [3, -1]).unwrap();
    assert_eq!(e.project.images[0].pos, [5.0, 2.0]);

    e.set_image_size(&id, 10.0, 2.5).unwrap();
    assert_eq!(e.project.images[0].size, [10.0, 2.5]);

    e.set_image_file(&id, "media/dog.gif", Some([4.0, 4.0])).unwrap();
    assert_eq!(e.project.images[0].file, "media/dog.gif");
    assert_eq!(e.project.images[0].size, [4.0, 4.0]);

    e.delete_items(&[], &[], std::slice::from_ref(&id)).unwrap();
    assert!(e.project.images.is_empty());

    // Each op was one undo step.
    e.undo().unwrap(); // un-delete
    assert_eq!(e.project.images.len(), 1);
    e.undo().unwrap(); // un-set-file
    assert_eq!(e.project.images[0].file, "media/cat.png");
}

#[test]
fn image_size_is_clamped_sane() {
    let mut e = Editor::empty();
    let id = e.place_image([0.0, 0.0], "media/x.png", [0.0, 900.0]).unwrap();
    assert_eq!(e.project.images[0].size, [0.5, 60.0]);
    e.set_image_size(&id, f32::NAN, -3.0).unwrap();
    assert_eq!(e.project.images[0].size, [6.0, 0.5]);
}

#[test]
fn fragment_and_paste_carry_images() {
    let mut e = Editor::empty();
    e.place_component("nand", [0, 0]).unwrap();
    let img = e.place_image([0.0, 3.0], "media/cat.png", [4.0, 2.0]).unwrap();
    let frag = e.fragment(&["nand_1".into()], &[], std::slice::from_ref(&img));
    assert_eq!(frag.images.len(), 1);
    let (comps, _, imgs) = e.paste_fragment(&frag, [8, 0], true).unwrap();
    assert_eq!(comps.len(), 1);
    assert_eq!(imgs.len(), 1);
    assert_eq!(e.project.images.len(), 2);
    // The copy has a fresh id and shares the media file.
    assert_ne!(e.project.images[1].id, img);
    assert_eq!(e.project.images[1].file, "media/cat.png");
    assert_eq!(e.project.images[1].pos, [8.0, 3.0]);
}

#[test]
fn normalize_repairs_broken_images() {
    let mut project = Editor::empty().project;
    project.images = vec![
        crate::project::MediaImage {
            id: "img_1".into(),
            file: "media/a.png".into(),
            pos: [0.0, 0.0],
            size: [4.0, 3.0],
        },
        crate::project::MediaImage {
            id: "img_1".into(), // duplicate id
            file: "media/b.png".into(),
            pos: [f32::NAN, 2.0],
            size: [-5.0, 3.0], // broken size
        },
    ];
    let notes = normalize(&mut project);
    assert!(!notes.is_empty());
    assert_ne!(project.images[0].id, project.images[1].id);
    assert_eq!(project.images[1].size[0], 0.5);
    assert_eq!(project.images[1].pos[0], 0.0);
}
