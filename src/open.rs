//! Launching editors and the system file manager.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::store::Config;

/// Opens `path` with the configured editor, or the file manager if none is set.
pub fn open_project(config: &Config, path: &Path) -> io::Result<()> {
    let editor = config.editor.as_deref().unwrap_or("").trim();
    if editor.is_empty() {
        return reveal(path);
    }
    let mut command = command_for(editor)?;
    command.args(&config.editor_args).arg(path).current_dir(path);
    spawn(command)
}

pub struct Editor {
    pub name: &'static str,
    /// Value stored in `config.editor`: the bare name if it's on PATH, else a full path.
    pub command: String,
}

/// GUI editors found on this machine, via PATH or their usual install location.
pub fn detect_editors() -> Vec<Editor> {
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
            found.push(Editor { name, command });
        }
    }
    found
}

fn same_program(a: &str, b: &str) -> bool {
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
            path.is_file().then_some(path)
        })
    })
}
