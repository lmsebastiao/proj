//! Finding installed editors and naming editor commands.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

use crate::open::which;

#[derive(Clone)]
pub struct Editor {
    pub name: String,
    /// Value stored in `config.editor`: the bare name if it's on PATH, else a full path.
    pub command: String,
}

static DETECTED: Mutex<Option<Vec<Editor>>> = Mutex::new(None);

/// Cached [`detect_editors`]. `refresh` rescans, e.g. when showing the editor list.
pub fn detected_editors(refresh: bool) -> Vec<Editor> {
    let mut cache = DETECTED.lock().unwrap_or_else(|e| e.into_inner());
    if refresh || cache.is_none() {
        *cache = Some(detect_editors());
    }
    cache.clone().unwrap_or_default()
}

/// GUI editors found on this machine, via PATH or their usual install location.
fn detect_editors() -> Vec<Editor> {
    // (display name, CLI name, fallback locations relative to the given base dir)
    #[rustfmt::skip]
    const KNOWN: &[(&str, &str, &[&str])] = &[
        ("Zed", "zed", &[
            "%LOCALAPPDATA%/Programs/Zed/bin/zed.exe",
            "/Applications/Zed.app/Contents/MacOS/cli",
        ]),
        ("Zed Preview", "zed-preview", &[
            "%LOCALAPPDATA%/Programs/Zed Preview/bin/zed.exe",
            "/Applications/Zed Preview.app/Contents/MacOS/cli",
        ]),
        ("Visual Studio Code", "code", &[
            "%LOCALAPPDATA%/Programs/Microsoft VS Code/bin/code.cmd",
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
        ]),
        ("Cursor", "cursor", &[
            "%LOCALAPPDATA%/Programs/cursor/resources/app/bin/cursor.cmd",
            "/Applications/Cursor.app/Contents/Resources/app/bin/cursor",
        ]),
        ("Windsurf", "windsurf", &[
            "%LOCALAPPDATA%/Programs/Windsurf/bin/windsurf.cmd",
            "/Applications/Windsurf.app/Contents/Resources/app/bin/windsurf",
        ]),
        ("VSCodium", "codium", &["%LOCALAPPDATA%/Programs/VSCodium/bin/codium.cmd"]),
        ("Sublime Text", "subl", &[
            "C:/Program Files/Sublime Text/subl.exe",
            "/Applications/Sublime Text.app/Contents/SharedSupport/bin/subl",
        ]),
        ("IntelliJ IDEA", "idea", &[]),
        ("Fleet", "fleet", &[]),
    ];
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let mut found: Vec<Editor> = Vec::new();
    for &(name, cli, locations) in KNOWN {
        let command = if which(cli).is_some() {
            Some(cli.to_string())
        } else {
            locations
                .iter()
                .filter(|l| !(local_app_data.is_empty() && l.contains("%LOCALAPPDATA%")))
                .map(|l| PathBuf::from(l.replace("%LOCALAPPDATA%", &local_app_data)))
                .find(|p| p.is_file())
                // Normalise separators ("C:\Users\me/Programs/..." -> all native).
                .map(|p| p.components().collect::<PathBuf>())
                .map(|p| p.to_string_lossy().into_owned())
        };
        // Skip duplicates, e.g. Zed Preview resolving to the same binary as Zed.
        if let Some(command) = command
            && !found.iter().any(|e| same_program(&e.command, &command))
        {
            found.push(Editor {
                name: name.into(),
                command,
            });
        }
    }
    found.extend(visual_studio());
    found
}

/// Visual Studio installs: `<Program Files>\Microsoft Visual Studio\<version>\<edition>\Common7\IDE\devenv.exe`.
fn visual_studio() -> Vec<Editor> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut found = Vec::new();
    for root in [
        r"C:\Program Files\Microsoft Visual Studio",
        r"C:\Program Files (x86)\Microsoft Visual Studio",
    ] {
        let mut versions: Vec<PathBuf> = std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        // Newest first.
        versions.sort_by(|a, b| b.cmp(a));
        for version in versions {
            for edition in std::fs::read_dir(&version).into_iter().flatten().flatten() {
                let devenv = edition.path().join(r"Common7\IDE\devenv.exe");
                if !devenv.is_file() {
                    continue;
                }
                let version = version
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                // VS 2026 installs into "18"; older releases use the year.
                let year = if version == "18" {
                    "2026".to_string()
                } else {
                    version
                };
                found.push(Editor {
                    name: format!("Visual Studio {year}"),
                    command: devenv.to_string_lossy().into_owned(),
                });
            }
        }
    }
    found
}

/// Display name for an editor command: its detected name, else the program name.
pub fn editor_name(command: &str, detected: &[Editor]) -> String {
    if command.trim().is_empty() {
        return "the file manager".into();
    }
    detected
        .iter()
        .find(|e| e.command == command)
        .or_else(|| detected.iter().find(|e| same_program(&e.command, command)))
        .map(|e| e.name.clone())
        .unwrap_or_else(|| editor_label(command))
}

pub fn same_program(a: &str, b: &str) -> bool {
    let resolve = |s: &str| which(s).unwrap_or_else(|| PathBuf::from(s));
    resolve(a) == resolve(b)
}

/// Display name for a configured editor command.
pub fn editor_label(command: &str) -> String {
    if command.trim().is_empty() {
        return "the file manager".into();
    }
    Path::new(command)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| command.to_string())
}
