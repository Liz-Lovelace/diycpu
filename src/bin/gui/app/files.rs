//! Projects on disk: the projects folder, save/open/new, write-through
//! autosave, media import, and the first-launch starter project.

use std::path::{Path, PathBuf};

use diycpu::editor::{Editor, SheetRef};
use diycpu::project::{PortRef, Project};


use super::*;


impl App {
    /// Save to the current file, or fall through to Save As when the
    /// project has never been saved.
    pub fn save(&mut self) {
        match self.file_path.clone() {
            Some(path) => self.write_to(path),
            None => self.save_as(),
        }
    }

    /// "Save as" forks the project: a new directory in the projects folder
    /// under a typed name, media files included.
    pub fn save_as(&mut self) {
        if projects_base_dir().is_none() {
            // No home folder to keep projects in — fall back to a raw
            // file dialog.
            let picked = rfd::FileDialog::new()
                .add_filter("circuit", &["json"])
                .set_file_name(format!("{}.json", sanitize_name(&self.editor.project.name)))
                .save_file();
            match picked {
                Some(path) => self.write_to(path),
                None => self.info("Save cancelled"),
            }
            return;
        }
        self.gesture = Gesture::Idle;
        self.prompt = Some(Prompt {
            title: "Save a copy as (project name)".into(),
            value: sanitize_name(&self.editor.project.name),
            action: PromptAction::SaveCopyAs,
            identifier: true,
        });
    }

    /// Create `<projects>/<name>/`, copy every referenced media file into
    /// it, and write the project there. Returns the new project.json path.
    pub(super) fn save_copy_into(&mut self, name: &str) -> Result<PathBuf, String> {
        let base = projects_base_dir().ok_or("No home folder found")?;
        std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let dir = create_project_dir(&base, name)?;
        if let Some(old_dir) = self.project_dir() {
            let mut files: Vec<String> = self
                .editor
                .project
                .images
                .iter()
                .map(|i| i.file.clone())
                .collect();
            for a in &self.editor.project.abstractions {
                files.extend(a.images.iter().map(|i| i.file.clone()));
            }
            files.sort();
            files.dedup();
            for rel in files {
                let src = old_dir.join(&rel);
                let dst = dir.join(&rel);
                if src.exists() {
                    if let Some(parent) = dst.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::copy(&src, &dst);
                }
            }
        }
        let _ = self.editor.rename_project(name);
        let path = dir.join("project.json");
        self.persist(&path)?;
        Ok(path)
    }

    /// Write-through persistence: every committed change lands on disk as
    /// soon as the project has a file. Called by the shell after events.
    pub fn autosave(&mut self) {
        if !self.editor.dirty {
            return;
        }
        let Some(path) = self.file_path.clone() else {
            return; // nowhere to save yet — Save As picks the home
        };
        match self.persist(&path) {
            Ok(()) => {}
            Err(e) => self.error(format!("Autosave failed: {e}")),
        }
    }

    pub(super) fn persist(&mut self, path: &std::path::Path) -> Result<(), String> {
        serde_json::to_string_pretty(&self.editor.project)
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(path, json + "\n").map_err(|e| e.to_string()))
            .map(|()| self.editor.dirty = false)
    }

    pub(super) fn write_to(&mut self, path: PathBuf) {
        match self.persist(&path) {
            Ok(()) => {
                self.file_path = Some(path.clone());
                self.info(format!("Saved {}", path.display()));
            }
            Err(e) => self.error(format!("Save failed: {e}")),
        }
    }

    /// True when it's safe to discard the document (clean, or confirmed).
    pub(super) fn confirm_discard(&mut self, intent: DiscardIntent, again: &str) -> bool {
        if !self.editor.dirty || self.pending_discard == Some(intent) {
            self.pending_discard = None;
            return true;
        }
        self.pending_discard = Some(intent);
        self.error(format!("Unsaved changes — {again} again to discard them"));
        false
    }

    pub(super) fn new_project(&mut self) {
        if projects_base_dir().is_none() {
            // No projects folder available: the old in-memory new.
            if !self.confirm_discard(DiscardIntent::New, "press Ctrl+N") {
                return;
            }
            self.editor = Editor::empty();
            self.file_path = None;
            self.reset_to_fresh_view();
            self.info("New circuit");
            return;
        }
        // The current project is autosaved; a new one just needs a name.
        self.gesture = Gesture::Idle;
        self.prompt = Some(Prompt {
            title: "New project name".into(),
            value: String::new(),
            action: PromptAction::NewProject,
            identifier: true,
        });
    }

    pub(super) fn reset_to_fresh_view(&mut self) {
        self.view_root = SheetRef::Top;
        self.path.clear();
        self.selection.clear();
        self.gesture = Gesture::Idle;
        self.mode = Mode::Edit;
        self.playing = false;
        self.pending_fit = true;
    }

    /// The window wants to close. True = allow it. A dirty never-saved
    /// project asks for one more click (everything else autosaves).
    pub fn request_close(&mut self) -> bool {
        if self.file_path.is_some() {
            self.autosave();
            return true;
        }
        self.confirm_discard(DiscardIntent::Close, "close")
    }

    pub fn open(&mut self) {
        if !self.confirm_discard(DiscardIntent::Open, "choose Open") {
            return;
        }
        // Projects are folders; picking one opens its project.json.
        // (A bare .json can still be opened from the command line.)
        let mut dialog = rfd::FileDialog::new();
        if let Some(base) = projects_base_dir() {
            let _ = std::fs::create_dir_all(&base);
            dialog = dialog.set_directory(&base);
        }
        let Some(dir) = dialog.pick_folder() else {
            self.info("Open cancelled");
            return;
        };
        let path = if dir.join("project.json").exists() {
            dir.join("project.json")
        } else if dir.extension().is_some_and(|e| e == "json") {
            dir
        } else {
            self.error(format!(
                "{} has no project.json — pick a project folder",
                dir.display()
            ));
            return;
        };
        self.open_path(path);
    }

    /// Load a project file and point the session at it.
    pub(super) fn open_path(&mut self, path: PathBuf) {
        match load_project(&path) {
            Ok((project, notes)) => {
                self.editor = Editor::new(project);
                self.file_path = Some(path.clone());
                self.reset_to_fresh_view();
                if notes.is_empty() {
                    self.info(format!("Opened \"{}\"", self.editor.project.name));
                } else {
                    self.error(format!(
                        "Opened \"{}\" — repaired: {}",
                        self.editor.project.name,
                        notes.join("; ")
                    ));
                }
            }
            Err(e) => self.error(e),
        }
    }

    /// The directory the project file lives in — media paths are relative
    /// to it.
    pub fn project_dir(&self) -> Option<PathBuf> {
        Some(self.file_path.as_ref()?.parent()?.to_path_buf())
    }

    /// Absolute path of a project-relative media file.
    pub fn resolve_media(&self, rel: &str) -> Option<PathBuf> {
        Some(self.project_dir()?.join(rel))
    }

    /// Check a media candidate: exists, within the size cap, and a format
    /// we can actually decode.
    pub(super) fn validate_image_file(path: &Path) -> Result<(u32, u32), String> {
        let meta = std::fs::metadata(path)
            .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        if meta.len() > MAX_MEDIA_BYTES {
            return Err(format!(
                "That file is {:.1} MB — the limit is 20 MB",
                meta.len() as f64 / (1024.0 * 1024.0)
            ));
        }
        let reader = image::ImageReader::open(path)
            .map_err(|e| format!("Could not open {}: {e}", path.display()))?
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        use image::ImageFormat;
        match reader.format() {
            Some(
                ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP,
            ) => {}
            _ => {
                return Err(
                    "Unsupported format — png, jpeg, gif or webp (videos like webm aren't supported)"
                        .into(),
                )
            }
        }
        reader
            .into_dimensions()
            .map_err(|e| format!("Could not decode the image: {e}"))
    }

    /// Copy a file into the project's media folder (unless it's already
    /// there) and return its project-relative path.
    pub(super) fn import_media(&self, source: &Path) -> Result<String, String> {
        let dir = self
            .project_dir()
            .ok_or("Save the project first — images are copied into its folder")?;
        let media = dir.join("media");
        // Already one of ours? Reference it as-is.
        if source.starts_with(&media) {
            if let Some(name) = source.file_name() {
                return Ok(format!("media/{}", name.to_string_lossy()));
            }
        }
        std::fs::create_dir_all(&media).map_err(|e| e.to_string())?;
        let stem: String = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".into())
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let ext = source
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let mut name = format!("{stem}.{ext}");
        let mut n = 1;
        while media.join(&name).exists() {
            n += 1;
            name = format!("{stem}-{n}.{ext}");
        }
        std::fs::copy(source, media.join(&name)).map_err(|e| e.to_string())?;
        Ok(format!("media/{name}"))
    }

    /// Palette "image": pick a file first, then place it like a component.
    pub fn action_place_image(&mut self) {
        if self.project_dir().is_none() {
            self.error("Save the project first — images are copied into its folder");
            return;
        }
        let picked = rfd::FileDialog::new()
            .add_filter("images", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file();
        let Some(path) = picked else {
            self.info("Cancelled");
            return;
        };
        self.begin_image_placement(path);
    }

    /// Validate a picked file and arm placement (split from the picker so
    /// tests can drive it).
    pub fn begin_image_placement(&mut self, source: PathBuf) {
        match Self::validate_image_file(&source) {
            Ok((iw, ih)) => {
                let aspect = ih as f32 / iw.max(1) as f32;
                let w = 11.0f32;
                let h = (((w * aspect) * 2.0).round() / 2.0).clamp(1.0, 24.0);
                self.gesture = Gesture::PlacingImage {
                    source,
                    size: [w, h],
                };
                self.info("Placing the image — click the canvas, Esc cancels");
            }
            Err(e) => self.error(e),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscardIntent {
    New,
    Open,
    Close,
}

/// A string safe to use as a project directory name.
pub(crate) fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "circuit".into()
    } else {
        cleaned
    }
}

/// Where projects live: the XDG data directory — `~/.local/share/diycpu`
/// on Linux (respects $XDG_DATA_HOME) — falling back to `~/.diycpu`.
/// None only when no home directory can be found at all.
pub fn projects_base_dir() -> Option<PathBuf> {
    if let Some(data) = dirs::data_dir() {
        return Some(data.join("diycpu"));
    }
    Some(dirs::home_dir()?.join(".diycpu"))
}

/// Create `<base>/<name>/` (with its media folder). The name must be free.
pub fn create_project_dir(base: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Project names use letters, digits, _ and -".into());
    }
    let dir = base.join(name);
    if dir.exists() {
        return Err(format!("A project called \"{name}\" already exists"));
    }
    std::fs::create_dir_all(dir.join("media")).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// What to open when the app starts without a path: the most recently
/// modified project in the folder — or, on the very first launch, a fresh
/// starter project created right there.
pub fn default_project_path(base: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(base).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let candidate = entry.path().join("project.json");
        let Ok(meta) = std::fs::metadata(&candidate) else {
            continue;
        };
        let t = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if newest.as_ref().is_none_or(|(bt, _)| t > *bt) {
            newest = Some((t, candidate));
        }
    }
    if let Some((_, path)) = newest {
        return Ok(path);
    }
    let dir = create_project_dir(base, "welcome")?;
    let path = dir.join("project.json");
    let json =
        serde_json::to_string_pretty(&starter_project()).map_err(|e| e.to_string())?;
    std::fs::write(&path, json + "\n").map_err(|e| e.to_string())?;
    Ok(path)
}

/// The first thing a new user sees: a little blinker with a welcome note,
/// built through the same editor ops the GUI uses so it can never drift
/// from the format.
pub fn starter_project() -> Project {
    pub(super) fn build() -> Result<Project, diycpu::editor::EditError> {
        let port = |c: &str, p: &str| PortRef {
            component_id: c.into(),
            port_name: p.into(),
        };
        let mut ed = Editor::empty();
        ed.rename_project("welcome")?;
        ed.place_component("clk", [1, 6])?; // clk_1
        ed.place_component("dff", [6, 2])?; // dff_1
        ed.place_component("nand", [12, 2])?; // nand_1
        // dff.out feeds both NAND inputs: the NAND is an inverter.
        ed.connect(port("dff_1", "out"), port("nand_1", "a"), &[])?;
        ed.connect(
            port("dff_1", "out"),
            port("nand_1", "b"),
            &[[10.5, 2.5], [10.5, 3.5]],
        )?;
        // ...and the inverse loops back into the flip-flop, around the top.
        ed.connect(
            port("nand_1", "out"),
            port("dff_1", "d"),
            &[[16.5, 2.5], [16.5, 0.5], [4.5, 0.5], [4.5, 2.5]],
        )?;
        ed.connect(
            port("clk_1", "out"),
            port("dff_1", "clk"),
            &[[4.5, 6.5], [4.5, 3.5]],
        )?;
        ed.place_note(
            [10.5, 6.0],
            "Welcome! This loop blinks: the NAND feeds the flip-flop its own inverse. Press Space to run. Scroll zooms, right-drag pans.",
        )?;
        Ok(ed.project)
    }
    build().unwrap_or_else(|_| Editor::empty().project)
}

pub fn load_project(path: &std::path::Path) -> Result<(Project, Vec<String>), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let mut project: Project = serde_json::from_str(&text)
        .map_err(|e| format!("Could not parse {}: {e}", path.display()))?;
    let notes = diycpu::editor::normalize(&mut project);
    Ok((project, notes))
}

/// Media files are capped so projects stay portable.
pub const MAX_MEDIA_BYTES: u64 = 20 * 1024 * 1024;
