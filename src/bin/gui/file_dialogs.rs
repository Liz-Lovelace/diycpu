use std::path::PathBuf;
use std::process::Command;

/// Show a native "Open File" dialog using zenity.
/// Returns the selected file path, or None if the user cancelled.
pub fn open_file_dialog() -> Option<PathBuf> {
    let output = Command::new("zenity")
        .args([
            "--file-selection",
            "--title=Open Circuit YAML",
            "--file-filter=YAML files | *.yaml *.yml",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        let selected_path = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if selected_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(selected_path))
        }
    } else {
        None
    }
}

/// Show a native "Save File" dialog using zenity.
/// Returns the selected file path, or None if the user cancelled.
pub fn save_file_dialog() -> Option<PathBuf> {
    let output = Command::new("zenity")
        .args([
            "--file-selection",
            "--save",
            "--title=Save Circuit YAML",
            "--file-filter=YAML files | *.yaml *.yml",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        let selected_path = String::from_utf8(output.stdout).ok()?.trim().to_string();
        if selected_path.is_empty() {
            None
        } else {
            Some(PathBuf::from(selected_path))
        }
    } else {
        None
    }
}
