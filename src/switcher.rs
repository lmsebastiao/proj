//! The window switcher: open editor windows, and which project each one shows.

use std::path::{Path, PathBuf};

use crate::{
    config::Config,
    editors,
    open::{self, which},
    platform::{self, WindowRef},
    store::{Db, Project},
};

/// An open window of one of the user's editors.
#[derive(Clone)]
pub struct EditorWindow {
    pub window: WindowRef,
    /// As the editor sets it, e.g. "proj — main.rs".
    pub title: String,
    /// e.g. "Zed", "Visual Studio Code".
    pub editor: String,
    /// The program that owns it, for its icon.
    pub exe: PathBuf,
    /// Lowercase program name, e.g. "zed", "devenv".
    process: String,
    /// Its place front to back when the list was made: 0 was used last.
    z: usize,
    /// It was the window in front then, the one being switched away from.
    front: bool,
}

impl EditorWindow {
    /// The order to switch to a project's windows in: the one used last
    /// first, but the one being switched away from last.
    pub fn rank(&self) -> (bool, usize) {
        (self.front, self.z)
    }
}

/// The switcher's rows, as indexes into `windows` (in the switcher's order):
/// windows of the same project (`projects_of`) share a row where the first of
/// them is, best to switch to first (see [`EditorWindow::rank`]); the others
/// get a row each.
pub fn rows(windows: &[EditorWindow], projects_of: &[Option<usize>]) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    for (w, project) in projects_of.iter().enumerate() {
        let shared =
            project.and_then(|p| rows.iter().position(|row| projects_of[row[0]] == Some(p)));
        match shared {
            Some(row) => rows[row].push(w),
            None => rows.push(vec![w]),
        }
    }
    for row in &mut rows {
        row.sort_by_key(|&w| windows[w].rank());
    }
    rows
}

/// A shortcut from the config for display, like the palette's keys:
/// "ctrl+alt+Backslash" → "ctrl-alt-\".
pub fn shortcut_label(shortcut: &str) -> String {
    let (mods, key) = shortcut.rsplit_once('+').unwrap_or(("", shortcut));
    let key = match key.to_lowercase().as_str() {
        "backquote" => "`".to_string(),
        "backslash" => "\\".to_string(),
        "quote" => "'".to_string(),
        "semicolon" => ";".to_string(),
        "slash" => "/".to_string(),
        "bracketleft" => "[".to_string(),
        "bracketright" => "]".to_string(),
        "minus" => "-".to_string(),
        "equal" => "=".to_string(),
        other => other.to_string(),
    };
    if mods.is_empty() {
        key
    } else {
        format!("{}-{key}", mods.replace('+', "-").to_lowercase())
    }
}

/// Windows of the detected and configured editors, front to back.
pub fn editor_windows(config: &Config, db: &Db) -> Vec<EditorWindow> {
    let detected = editors::detected_editors(false);
    // (program path, display name) for every editor proj knows about.
    let known: Vec<(PathBuf, String)> = detected
        .iter()
        .map(|e| e.command.clone())
        .chain(config.editor.iter().cloned())
        .chain(db.editors.values().flatten().cloned())
        .filter(|command| !command.trim().is_empty())
        .map(|command| {
            let path = which(&command).unwrap_or_else(|| PathBuf::from(&command));
            // macOS: `zed` on PATH is a link into Zed.app.
            #[cfg(target_os = "macos")]
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            let name = editors::editor_name(&command, &detected);
            (path, name)
        })
        .collect();
    // After listing them: on macOS the window in front is one of that list.
    let windows = platform::top_windows();
    let front = platform::foreground_window();
    windows
        .into_iter()
        .filter_map(|window| {
            let process = stem(&window.exe);
            let editor = editor_of(&window.exe, &process, &known)?;
            Some((window, process, editor))
        })
        .enumerate()
        .map(|(z, (window, process, editor))| EditorWindow {
            front: Some(window.window) == front,
            window: window.window,
            title: window.title,
            editor,
            exe: window.exe,
            process,
            z,
        })
        .collect()
}

/// The editor a program belongs to. The command is often a launcher (`bin\zed.exe`,
/// `code.cmd`) next to the real program, so match on the name, or on macOS on
/// the app both are in, then prefer the install that shares the most of the
/// program's path (Zed vs Zed Preview).
fn editor_of(exe: &Path, process: &str, known: &[(PathBuf, String)]) -> Option<String> {
    known
        .iter()
        .filter(|(command, _)| {
            let name = stem(command);
            name == process
                || editors::GUI_NAMES
                    .iter()
                    .any(|&(cli, gui)| cli == name && gui == process)
                || app_bundle(exe).is_some_and(|app| Some(app) == app_bundle(command))
        })
        .max_by_key(|(command, _)| {
            exe.components()
                .zip(command.components())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .map(|(_, name)| name.clone())
}

/// The macOS app a program is in: `/Applications/Zed.app` for
/// `/Applications/Zed.app/Contents/MacOS/cli`.
fn app_bundle(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// The project a window shows, going by its title: editors put the
/// folder name there ("proj — main.rs", "main.rs - proj - Visual Studio Code"),
/// Zed lists a workspace's folders ("app, shared-sdk"), and Visual Studio shows
/// the solution name.
impl EditorWindow {
    /// Index of the project in `projects` this window shows.
    pub fn project(&self, projects: &[Project]) -> Option<usize> {
        project_for(&self.title, &self.process, projects)
    }
}

fn project_for(title: &str, process: &str, projects: &[Project]) -> Option<usize> {
    let parts = title_parts(title);
    let has = |name: &str| parts.iter().any(|p| p.eq_ignore_ascii_case(name));
    let folder = |path: &Path| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    // Workspaces first, so "app, shared-sdk" isn't taken for "app" alone.
    let workspace = projects.iter().position(|project| {
        project.is_workspace()
            && parts.iter().any(|part| {
                let mut listed: Vec<String> = part.split(", ").map(str::to_lowercase).collect();
                let mut folders: Vec<String> = project
                    .paths()
                    .iter()
                    .map(|p| folder(p).to_lowercase())
                    .collect();
                listed.sort();
                folders.sort();
                listed == folders
            })
    });
    workspace.or_else(|| {
        projects.iter().position(|project| {
            !project.is_workspace()
                && (has(&project.name)
                    || has(&folder(&project.path))
                    || (process == "devenv"
                        && open::find_solution(&project.path).is_some_and(|sln| has(&stem(&sln)))))
        })
    })
}

/// "● main.rs - proj (Running) - Visual Studio Code" → ["main.rs", "proj", "Visual Studio Code"].
fn title_parts(title: &str) -> Vec<String> {
    title
        .replace('\u{a0}', " ")
        .split(" — ")
        .flat_map(|part| part.split(" - "))
        .flat_map(|part| part.split(" – "))
        .map(|part| {
            let part = part.trim().trim_start_matches('●').trim();
            // "proj (Running)", "proj (Workspace)", "proj [Administrator]".
            match part.rfind(' ') {
                Some(space)
                    if part.ends_with([')', ']']) && part[space + 1..].starts_with(['(', '[']) =>
                {
                    part[..space].to_string()
                }
                _ => part.to_string(),
            }
        })
        .filter(|part| !part.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, paths: &[&str]) -> Project {
        Project {
            name: name.into(),
            path: paths[0].into(),
            extra: paths[1..].iter().map(PathBuf::from).collect(),
            manual: true,
            ..Project::default()
        }
    }

    #[test]
    fn labels_shortcuts() {
        assert_eq!(shortcut_label("ctrl+alt+Backslash"), "ctrl-alt-\\");
        assert_eq!(shortcut_label("Alt+Q"), "alt-q");
        assert_eq!(shortcut_label("F8"), "f8");
    }

    #[test]
    fn splits_titles() {
        assert_eq!(title_parts("proj — items.rs"), ["proj", "items.rs"]);
        assert_eq!(
            title_parts("● main.rs - api-v2 (Workspace) - Visual Studio Code"),
            ["main.rs", "api-v2", "Visual Studio Code"]
        );
        assert_eq!(
            title_parts("TomiManager (Running) - Microsoft Visual Studio"),
            ["TomiManager", "Microsoft Visual Studio"]
        );
    }

    #[test]
    fn matches_windows_to_projects() {
        let projects = vec![
            project("app", &["/r/app"]),
            project("Client", &["/r/interactive-v2"]),
            project("app + shared-sdk", &["/r/app", "/r/shared-sdk"]),
        ];
        let find = |title: &str| project_for(title, "zed", &projects);
        assert_eq!(find("app — main.rs"), Some(0));
        assert_eq!(find("interactive-v2 — main.rs"), Some(1), "by folder");
        assert_eq!(find("Client"), Some(1), "by its custom name");
        assert_eq!(find("shared-sdk, app — lib.rs"), Some(2), "a Zed workspace");
        assert_eq!(find("main.rs - app - Visual Studio Code"), Some(0));
        assert_eq!(find("empty project"), None);
    }

    #[test]
    fn tells_editor_installs_apart() {
        let known = vec![
            (PathBuf::from(r"C:\P\Zed\bin\zed.exe"), "Zed".to_string()),
            (
                PathBuf::from(r"C:\P\Zed Preview\bin\zed.exe"),
                "Zed Preview".to_string(),
            ),
            (
                PathBuf::from(r"C:\P\VSCodium\bin\codium.cmd"),
                "VSCodium".to_string(),
            ),
        ];
        let editor = |exe: &str| {
            let exe = PathBuf::from(exe);
            editor_of(&exe, &stem(&exe), &known)
        };
        assert_eq!(
            editor(r"C:\P\Zed Preview\Zed.exe").as_deref(),
            Some("Zed Preview")
        );
        assert_eq!(editor(r"C:\P\Zed\Zed.exe").as_deref(), Some("Zed"));
        assert_eq!(
            editor(r"C:\P\VSCodium\VSCodium.exe").as_deref(),
            Some("VSCodium")
        );
        assert_eq!(editor(r"C:\Windows\explorer.exe"), None);

        // macOS: the CLI and the app's own program share the app.
        let known = vec![
            (
                PathBuf::from("/Applications/Zed.app/Contents/MacOS/cli"),
                "Zed".to_string(),
            ),
            (
                PathBuf::from(
                    "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                ),
                "Visual Studio Code".to_string(),
            ),
        ];
        let editor = |exe: &str| {
            let exe = PathBuf::from(exe);
            editor_of(&exe, &stem(&exe), &known)
        };
        assert_eq!(
            editor("/Applications/Zed.app/Contents/MacOS/zed").as_deref(),
            Some("Zed")
        );
        assert_eq!(
            editor("/Applications/Visual Studio Code.app/Contents/MacOS/Electron").as_deref(),
            Some("Visual Studio Code")
        );
        assert_eq!(
            editor("/Applications/Safari.app/Contents/MacOS/Safari"),
            None
        );
    }

    #[test]
    fn a_project_s_windows_share_a_row() {
        // In the switcher's order; z is front to back, and window 2 was in front.
        let window = |id: isize, z: usize| EditorWindow {
            window: WindowRef::test(id),
            title: format!("window {id}"),
            editor: "Zed".into(),
            exe: PathBuf::new(),
            process: "zed".into(),
            z,
            front: z == 0,
        };
        let windows = [window(0, 3), window(1, 1), window(2, 0), window(3, 2)];
        let projects_of = [Some(7), None, Some(7), Some(7)];
        assert_eq!(
            rows(&windows, &projects_of),
            [vec![3, 0, 2], vec![1]],
            "where its first window is; the one switched away from last"
        );
        assert_eq!(rows(&[], &[]), Vec::<Vec<usize>>::new());
    }
}
