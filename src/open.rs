//! Launching editors and the system file manager.

use std::{
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::store::Config;

/// Opens `path` with the configured editor, or the file manager if none is set.
pub fn open_project(config: &Config, path: &Path) -> io::Result<()> {
    if config.editor.trim().is_empty() {
        return reveal(path);
    }
    let mut command = command_for(config.editor.trim())?;
    command.args(&config.editor_args).arg(path).current_dir(path);
    spawn(command)
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
