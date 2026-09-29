//! Launching editors and the system file manager.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

use crate::store::Config;

/// Opens `path` with the global editor, or the file manager if none is set.
pub fn open_project(config: &Config, path: &Path) -> io::Result<()> {
    open_with(config, config.editor.as_deref().unwrap_or(""), &[path.to_path_buf()])
}

/// Opens `paths` with `editor` ("" = file manager). Several paths open as one
/// window in editors that support it (Zed, VS Code). `editor_args` belong to the
/// global editor, so they're only passed to that one.
pub fn open_with(config: &Config, editor: &str, paths: &[PathBuf]) -> io::Result<()> {
    let Some(first) = paths.first() else {
        return Ok(());
    };
    let editor = editor.trim();
    if editor.is_empty() {
        return reveal(first);
    }
    let mut command = command_for(editor)?;
    if config.editor.as_deref().map(str::trim) == Some(editor) {
        command.args(&config.editor_args);
    }
    if opens_solutions(editor) {
        // Visual Studio and Rider open one solution, not folders.
        let solution = first
            .is_dir()
            .then(|| find_solution(first))
            .flatten()
            .unwrap_or_else(|| first.clone());
        command.arg(solution);
    } else {
        command.args(paths);
    }
    if first.is_dir() {
        command.current_dir(first);
    }
    spawn(command)
}

fn opens_solutions(editor: &str) -> bool {
    let stem = Path::new(editor)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(stem.as_str(), "devenv" | "rider" | "rider64")
}

/// A `.sln`/`.slnx` in the project root, or else one level down.
fn find_solution(dir: &Path) -> Option<PathBuf> {
    let solutions_in = |dir: &Path| -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| {
                    e.eq_ignore_ascii_case("sln") || e.eq_ignore_ascii_case("slnx")
                })
            })
            .collect();
        found.sort();
        found
    };
    if let Some(solution) = solutions_in(dir).into_iter().next() {
        return Some(solution);
    }
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && !p
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        })
        .collect();
    subdirs.sort();
    subdirs
        .iter()
        .find_map(|sub| solutions_in(sub).into_iter().next())
}

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

/// Opens a terminal in `path`.
pub fn open_terminal(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        if which("wt").is_some() {
            let mut command = Command::new("wt");
            command.arg("-d").arg(path);
            return spawn(command);
        }
        // No Windows Terminal: a plain console, which needs its own window.
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        Command::new("cmd")
            .arg("/K")
            .current_dir(path)
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map(drop)
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        command.args(["-a", "Terminal"]).arg(path);
        spawn(command)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let terminal = [
            "x-terminal-emulator",
            "gnome-terminal",
            "konsole",
            "alacritty",
            "kitty",
            "xterm",
        ]
        .into_iter()
        .find(|t| which(t).is_some())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no terminal found"))?;
        let mut command = Command::new(terminal);
        command.current_dir(path);
        spawn(command)
    }
}

/// Opens `path` in the system file manager.
pub fn reveal(path: &Path) -> io::Result<()> {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(program);
    command.arg(path);
    spawn(command)
}

fn command_for(program: &str) -> io::Result<Command> {
    let resolved = which(program).unwrap_or_else(|| PathBuf::from(program));
    let is_script = resolved
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"));
    if cfg!(target_os = "windows") && is_script {
        // `code`, `subl` etc. ship as .cmd shims on Windows which need cmd.exe.
        let mut command = Command::new("cmd");
        command.arg("/C").arg(resolved);
        Ok(command)
    } else {
        Ok(Command::new(resolved))
    }
}

fn spawn(mut command: Command) -> io::Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // The child is intentionally not waited on; it outlives the launcher.
    command.spawn().map(drop)
}

/// Finds `program` on PATH (honouring PATHEXT on Windows).
pub fn which(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|e| !e.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        exts.iter().find_map(|ext| {
            let path = dir.join(format!("{program}{ext}"));
            // symlink_metadata: Windows app aliases (wt.exe in WindowsApps) are
            // reparse points that `is_file()` can't follow.
            path.symlink_metadata()
                .is_ok_and(|m| !m.is_dir())
                .then_some(path)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn solution_detection() {
        assert!(opens_solutions(
            r"C:\Program Files\Microsoft Visual Studio\18\Community\Common7\IDE\devenv.exe"
        ));
        assert!(!opens_solutions("zed"));

        let dir = std::env::temp_dir().join(format!("proj-sln-{}", std::process::id()));
        fs::create_dir_all(dir.join("src")).unwrap();
        assert_eq!(find_solution(&dir), None);
        fs::write(dir.join("src/App.slnx"), "").unwrap();
        assert_eq!(find_solution(&dir), Some(dir.join("src/App.slnx")));
        fs::write(dir.join("Root.sln"), "").unwrap();
        assert_eq!(
            find_solution(&dir),
            Some(dir.join("Root.sln")),
            "root wins over subfolders"
        );
        fs::remove_dir_all(dir).ok();
    }
}
