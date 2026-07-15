use std::path::{Path, PathBuf};



use super::*;


fn temp_base(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "diycpu-test-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn first_launch_creates_the_starter_then_reopens_it() {
    let base = temp_base("first");
    let path = default_project_path(&base).unwrap();
    assert!(path.ends_with("welcome/project.json"));
    assert!(base.join("welcome/media").is_dir());
    let (project, repairs) = load_project(&path).unwrap();
    assert!(repairs.is_empty(), "starter loads clean: {repairs:?}");
    assert_eq!(project.name, "welcome");
    assert!(!project.components.is_empty());
    assert!(!project.notes.is_empty());

    // Second launch: same project, no duplicate.
    let again = default_project_path(&base).unwrap();
    assert_eq!(again, path);
    assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn most_recently_touched_project_wins() {
    let base = temp_base("recent");
    let first = default_project_path(&base).unwrap();
    // A second project, saved later.
    let dir = create_project_dir(&base, "newer").unwrap();
    std::fs::write(dir.join("project.json"), "{}").unwrap();
    // File mtimes need to differ; nudge the newer one forward.
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    let f = std::fs::File::options()
        .write(true)
        .open(dir.join("project.json"))
        .unwrap();
    f.set_modified(later).unwrap();
    drop(f);
    let picked = default_project_path(&base).unwrap();
    assert!(picked.ends_with("newer/project.json"), "picked {picked:?}");
    assert_ne!(picked, first);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn project_dir_names_are_validated() {
    let base = temp_base("names");
    std::fs::create_dir_all(&base).unwrap();
    assert!(create_project_dir(&base, "ok-name_2").is_ok());
    assert!(create_project_dir(&base, "ok-name_2").is_err(), "taken");
    assert!(create_project_dir(&base, "no/slash").is_err());
    assert!(create_project_dir(&base, "").is_err());
    assert!(create_project_dir(&base, "no.dots").is_err());
    let _ = std::fs::remove_dir_all(&base);
}

fn tiny_png(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 4, 2);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(&[200u8; 4 * 2 * 4]).unwrap();
}

#[test]
fn placing_an_image_copies_it_into_the_project_media_folder() {
    let base = temp_base("place");
    std::fs::create_dir_all(&base).unwrap();
    let dir = create_project_dir(&base, "proj").unwrap();
    let source = base.join("photo.png");
    tiny_png(&source);

    let mut d = super::tests::Driver::new();
    d.app.file_path = Some(dir.join("project.json"));

    d.app.begin_image_placement(source.clone());
    assert!(
        matches!(d.app.gesture, Gesture::PlacingImage { .. }),
        "{}",
        d.app.status.text
    );
    d.click([6.0, 6.0]);
    assert_eq!(d.app.editor.project.images.len(), 1, "{}", d.app.status.text);
    let img = &d.app.editor.project.images[0];
    assert_eq!(img.file, "media/photo.png");
    // 4x2 source → width 11 cells, height keeps the 1:2 aspect.
    assert_eq!(img.size, [11.0, 5.5]);
    assert!(dir.join("media/photo.png").exists(), "file was copied");

    // Placing the same file again gets a fresh name, not a clobber.
    d.app.begin_image_placement(source);
    d.click([6.0, 12.0]);
    assert_eq!(d.app.editor.project.images[1].file, "media/photo-2.png");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn oversized_and_bogus_media_are_rejected() {
    let base = temp_base("reject");
    std::fs::create_dir_all(&base).unwrap();
    let mut d = super::tests::Driver::new();
    d.app.file_path = Some(base.join("project.json"));

    // Not an image at all (webm and friends land here too).
    let junk = base.join("video.webm");
    std::fs::write(&junk, b"\x1a\x45\xdf\xa3 not decodable").unwrap();
    d.app.begin_image_placement(junk);
    assert!(matches!(d.app.gesture, Gesture::Idle));
    assert!(d.app.status.is_error);
    assert!(d.app.status.text.contains("Unsupported"), "{}", d.app.status.text);

    // Over the 20 MB cap.
    let big = base.join("big.png");
    std::fs::write(&big, vec![0u8; 21 * 1024 * 1024]).unwrap();
    d.app.begin_image_placement(big);
    assert!(matches!(d.app.gesture, Gesture::Idle));
    assert!(d.app.status.text.contains("limit is 20 MB"), "{}", d.app.status.text);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn starter_project_actually_blinks() {
    let project = starter_project();
    let mut sim = crate::sim::SimSession::new(&project).expect("starter compiles");
    let dff = |s: &crate::sim::SimSession| {
        s.component_value_at(&[], "dff_1").unwrap()
    };
    let v0 = dff(&sim);
    // Two toggles = one rising edge = half a blink period.
    sim.step_forward();
    sim.step_forward();
    let v2 = dff(&sim);
    assert_ne!(v0, v2, "the blinker blinks");
}
