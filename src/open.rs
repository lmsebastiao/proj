//! Launching editors, terminals, the file manager and the browser.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::config::Config;

/// Opens `path` with the global editor, or the file manager if none is set.
pub fn open_project(config: &Config, path: &Path) -> io::Result<()> {
    open_with(
        config,
        config.editor.as_deref().unwrap_or(""),
        &[path.to_path_buf()],
    )
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

/// Opens `file` with `editor`, together with its project `folders` so it lands
/// in the project's window (Zed, VS Code). Solution-based IDEs get just the file;
/// no editor ("") uses the file's default app.
pub fn open_file(
    config: &Config,
    editor: &str,
    folders: &[PathBuf],
    file: &Path,
) -> io::Result<()> {
    let editor = editor.trim();
    if editor.is_empty() {
        return system_open(file.as_os_str());
    }
    let mut command = command_for(editor)?;
    if config.editor.as_deref().map(str::trim) == Some(editor) {
        command.args(&config.editor_args);
    }
    if !opens_solutions(editor) {
        command.args(folders);
    }
    command.arg(file);
    if let Some(dir) = folders.first().filter(|d| d.is_dir()) {
        command.current_dir(dir);
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
    system_open(path.as_os_str())
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) -> io::Result<()> {
    system_open(url.as_ref())
}

/// Hands `target` (a folder or URL) to the OS's default handler.
fn system_open(target: &std::ffi::OsStr) -> io::Result<()> {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut command = Command::new(program);
    command.arg(target);
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
