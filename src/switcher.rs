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
    /// Lowercase program name, e.g. "zed", "devenv".
    process: String,
}

/// GUI programs whose name differs from the command that opens them.
const GUI_NAMES: &[(&str, &str)] = &[
    ("codium", "vscodium"),
    ("subl", "sublime_text"),
    ("idea", "idea64"),
];

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
            let name = editors::editor_name(&command, &detected);
            (path, name)
        })
        .collect();
    platform::top_windows()
        .into_iter()
        .filter_map(|window| {
            let process = stem(&window.exe);
            let editor = editor_of(&window.exe, &process, &known)?;
            Some(EditorWindow {
                window: window.window,
                title: window.title,
                editor,
                process,
            })
        })
        .collect()
}

/// The editor a program belongs to. The command is often a launcher (`bin\zed.exe`,
/// `code.cmd`) next to the real program, so match on the name, then prefer the
/// install that shares the most of the program's path (Zed vs Zed Preview).
fn editor_of(exe: &Path, process: &str, known: &[(PathBuf, String)]) -> Option<String> {
    known
        .iter()
        .filter(|(command, _)| {
            let name = stem(command);
            name == process
                || GUI_NAMES
                    .iter()
                    .any(|&(cli, gui)| cli == name && gui == process)
        })
        .max_by_key(|(command, _)| {
            exe.components()
                .zip(command.components())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .map(|(_, name)| name.clone())
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
            branch: None,
            pinned: false,
            editor: None,
            manual: true,
            last_opened: 0,
        }
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
    }
}
