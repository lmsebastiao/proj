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
    /// The program the command starts, for its icon (see [`app_path`]).
    pub app: Option<PathBuf>,
}

impl Editor {
    pub fn new(name: String, command: String) -> Self {
        let app = app_path(&command);
        Self { name, command, app }
    }
}

/// GUI programs whose name differs from the command that opens them.
pub const GUI_NAMES: &[(&str, &str)] = &[
    ("codium", "vscodium"),
    ("subl", "sublime_text"),
    ("idea", "idea64"),
];

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
    // Each editor's own install, where it's in its usual place.
    let installs: Vec<Option<PathBuf>> = KNOWN
        .iter()
        .map(|&(_, _, locations)| {
            locations
                .iter()
                .filter(|l| !(local_app_data.is_empty() && l.contains("%LOCALAPPDATA%")))
                .map(|l| PathBuf::from(l.replace("%LOCALAPPDATA%", &local_app_data)))
                .find(|p| p.is_file())
                // Normalise separators ("C:\Users\me/Programs/..." -> all native).
                .map(|p| p.components().collect::<PathBuf>())
        })
        .collect();
    let mut found: Vec<Editor> = Vec::new();
    for (i, &(name, cli, _)) in KNOWN.iter().enumerate() {
        // The CLI name on PATH, unless it starts another editor's install:
        // Zed Preview's installer puts its own `zed` on PATH.
        let on_path = which(cli).is_some_and(|path| {
            !installs.iter().enumerate().any(|(j, install)| {
                j != i && install.as_deref().is_some_and(|o| same_path(o, &path))
            })
        });
        let command = if on_path {
            Some(cli.to_string())
        } else {
            installs[i]
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
        };
        // Skip duplicates, e.g. Zed Preview resolving to the same binary as Zed.
        if let Some(command) = command
            && !found.iter().any(|e| same_program(&e.command, &command))
        {
            found.push(Editor::new(name.into(), command));
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
                found.push(Editor::new(
                    format!("Visual Studio {year}"),
                    devenv.to_string_lossy().into_owned(),
                ));
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

/// The program an editor command starts, for its icon. Commands are often
/// launchers inside the install (`bin\code.cmd`, `resources\app\bin\cursor.cmd`),
/// so look a few folders up for the program of the same name, or its GUI name.
pub fn app_path(command: &str) -> Option<PathBuf> {
    if command.trim().is_empty() {
        return None;
    }
    let path = which(command).unwrap_or_else(|| PathBuf::from(command));
    if !cfg!(windows) {
        return path.is_file().then_some(path);
    }
    let stem = path.file_stem()?.to_string_lossy().to_lowercase();
    let names: Vec<&str> = GUI_NAMES
        .iter()
        .filter(|&&(cli, _)| cli == stem)
        .map(|&(_, gui)| gui)
        .chain([stem.as_str()])
        .collect();
    let is_exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    path.ancestors()
        .skip(1)
        .take(4)
        .find_map(|dir| {
            // Listed rather than joined, for the file's own casing ("Code.exe").
            let files: Vec<PathBuf> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .collect();
            names.iter().find_map(|name| {
                let exe = format!("{name}.exe");
                files
                    .iter()
                    .find(|f| {
                        f.file_name().is_some_and(|n| n.eq_ignore_ascii_case(&exe)) && f.is_file()
                    })
                    .cloned()
            })
        })
        .or_else(|| (is_exe && path.is_file()).then_some(path))
}

pub fn same_program(a: &str, b: &str) -> bool {
    let resolve = |s: &str| which(s).unwrap_or_else(|| PathBuf::from(s));
    same_path(&resolve(a), &resolve(b))
}

/// Whether two paths name the same file: on Windows, whatever their casing
/// ("Zed.exe" from PATH, "zed.exe" from the install list) or separators.
fn same_path(a: &Path, b: &Path) -> bool {
    let normal = |p: &Path| {
        let p: PathBuf = p.components().collect();
        let p = p.to_string_lossy().into_owned();
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    normal(a) == normal(b)
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_the_program_behind_a_launcher() {
        let dir = std::env::temp_dir().join(format!("proj-app-{}", std::process::id()));
        let file = |path: &str| {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "").unwrap();
            path
        };
        let cmd = file(r"VS Code\bin\code.cmd");
        let exe = file(r"VS Code\Code.exe");
        assert_eq!(app_path(&cmd.to_string_lossy()), Some(exe));
        let cmd = file(r"cursor\resources\app\bin\cursor.cmd");
        let exe = file(r"cursor\Cursor.exe");
        assert_eq!(app_path(&cmd.to_string_lossy()), Some(exe));
        let cmd = file(r"VSCodium\bin\codium.cmd");
        let exe = file(r"VSCodium\VSCodium.exe");
        assert_eq!(app_path(&cmd.to_string_lossy()), Some(exe), "by GUI name");
        let cli = file(r"Zed\bin\zed.exe");
        assert_eq!(app_path(&cli.to_string_lossy()), Some(cli), "itself");
        let script = file(r"tools\edit.cmd");
        assert_eq!(app_path(&script.to_string_lossy()), None);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn paths_match_whatever_their_casing() {
        assert!(same_path(
            Path::new(r"C:\Programs\Zed Preview\bin\Zed.exe"),
            Path::new(r"C:\Programs\Zed Preview\bin\zed.exe"),
        ));
        assert!(same_path(
            Path::new(r"C:\Programs\Zed/bin\zed.exe"),
            Path::new(r"C:\Programs\Zed\bin\zed.exe"),
        ));
        assert!(!same_path(
            Path::new(r"C:\Programs\Zed\bin\zed.exe"),
            Path::new(r"C:\Programs\Zed Preview\bin\zed.exe"),
        ));
    }
}
