//! What the list shows: item types, and each row's title, subtitle and details.

use std::path::PathBuf;

use crate::{
    config::{self, ThemeSetting},
    editors::Editor,
    launcher::UpdateState,
    paths, store, update,
};

use super::{Palette, secondary};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Projects,
    /// Choosing the global editor.
    Editors,
    /// Choosing an editor for one project (`Palette::open_with`).
    OpenWith,
    /// Inside a project's folders (`Palette::browse`).
    Browse,
    /// Typing a new name for an entry (`Palette::renaming`).
    Rename,
    /// The window switcher: open editor windows (`Palette::windows`).
    Switch,
    /// Ctrl-K: what can be done with one project (`Palette::actions_for`).
    Actions,
}

/// What the list currently shows. Commands appear when the query starts with `>`.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum List {
    Projects,
    Editors,
    OpenWith,
    Browse,
    Commands,
    /// Nothing: the search box holds the new name.
    Rename,
    Switch,
    Actions,
}

/// An entry in a project's actions menu (ctrl-k).
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ProjectAction {
    OpenWith,
    ShowInFileManager,
    Terminal,
    TogglePin,
    Rename,
    CopyPath,
    RepoPage,
    Remove,
}

pub(super) const PROJECT_ACTIONS: [ProjectAction; 8] = [
    ProjectAction::OpenWith,
    ProjectAction::ShowInFileManager,
    ProjectAction::Terminal,
    ProjectAction::TogglePin,
    ProjectAction::Rename,
    ProjectAction::CopyPath,
    ProjectAction::RepoPage,
    ProjectAction::Remove,
];

/// A git URL pasted into the search.
#[derive(Clone)]
pub(super) struct CloneTarget {
    pub(super) url: String,
    /// The folder `git clone` creates.
    pub(super) name: String,
    /// Where to clone it: the first `scan_dirs` folder, or `None` to ask.
    pub(super) into: Option<PathBuf>,
}

pub(super) enum EditorOption {
    Detected(Editor),
    Browse,
    FileManager,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum PaletteCommand {
    Autostart,
    AddProjects,
    ChangeEditor,
    /// Go through system → light → dark.
    Theme,
    OpenConfig,
    /// Check for an update, or install the one found.
    Update,
    Quit,
}

/// The `>` commands. `updates`: this copy can update itself (it was installed).
pub(super) fn commands(updates: bool) -> Vec<PaletteCommand> {
    [
        PaletteCommand::Autostart,
        PaletteCommand::AddProjects,
        PaletteCommand::ChangeEditor,
        PaletteCommand::Theme,
        PaletteCommand::OpenConfig,
    ]
    .into_iter()
    .chain(updates.then_some(PaletteCommand::Update))
    .chain([PaletteCommand::Quit])
    .collect()
}

pub(super) enum Target {
    Editor(String),
    FileManager,
    Terminal,
}

pub(super) struct Match {
    pub(super) ix: usize,
    pub(super) title_hl: Vec<usize>,
    pub(super) subtitle_hl: Vec<usize>,
}

/// Text shown on the right of a row.
pub(super) struct Meta {
    pub(super) top: Option<(String, u32)>,
    pub(super) bottom: Option<String>,
}

impl Palette {
    /// Title and subtitle of an item in the current list.
    pub(super) fn item_text(&self, ix: usize) -> (String, String) {
        match self.list() {
            List::Projects => {
                let project = &self.projects[ix];
                (project.name.clone(), project.location())
            }
            List::Browse => match &self.browse {
                Some(browse) => {
                    let entry = &browse.entries[ix];
                    let slash = if entry.is_dir { "/" } else { "" };
                    (
                        format!("{}{slash}", entry.name),
                        browse.relative(&entry.path),
                    )
                }
                None => Default::default(),
            },
            List::Editors | List::OpenWith => match &self.editors[ix] {
                EditorOption::Detected(editor) => (
                    editor.name.clone(),
                    paths::display_path(editor.command.as_ref()),
                ),
                EditorOption::Browse => {
                    let what = if self.mode == Mode::OpenWith {
                        " for this project"
                    } else {
                        ""
                    };
                    ("Other…".into(), format!("Pick any program{what}"))
                }
                EditorOption::FileManager => (
                    "No editor".into(),
                    "Open project folders in the file manager".into(),
                ),
            },
            List::Commands => {
                let m = secondary();
                match self.commands[ix] {
                    PaletteCommand::Autostart => (
                        format!(
                            "Start on login: {}",
                            if self.autostart { "on" } else { "off" }
                        ),
                        format!(
                            "Turn {} starting proj when you log in",
                            if self.autostart { "off" } else { "on" }
                        ),
                    ),
                    PaletteCommand::AddProjects => (
                        "Add projects…".into(),
                        format!("Pick one or more project folders · {m}-o"),
                    ),
                    PaletteCommand::ChangeEditor => (
                        "Change the default editor".into(),
                        format!(
                            "All projects open in {} unless they have their own",
                            self.name_of(self.config.editor.as_deref().unwrap_or(""))
                        ),
                    ),
                    PaletteCommand::OpenConfig => (
                        "Open config file".into(),
                        paths::display_path(&config::config_path()),
                    ),
                    PaletteCommand::Theme => {
                        let setting = self.config.theme;
                        let now = if self.theme.is_dark() {
                            "dark"
                        } else {
                            "light"
                        };
                        let title = match setting {
                            ThemeSetting::System => format!("Theme: system ({now})"),
                            _ => format!("Theme: {now}"),
                        };
                        let next = match setting.next() {
                            ThemeSetting::System => "follow the system setting".to_string(),
                            other => format!("use {}", other.as_str()),
                        };
                        (title, format!("↵ to {next}"))
                    }
                    PaletteCommand::Update => self.update_text(),
                    PaletteCommand::Quit => (
                        "Quit proj".into(),
                        format!("Stop the background launcher · {m}-q"),
                    ),
                }
            }
            // The project's name over the window's title ("proj — main.rs");
            // windows of no known project show just their title.
            List::Switch => {
                let window = &self.windows[ix];
                match self.window_projects[ix] {
                    Some(project) => (self.projects[project].name.clone(), window.title.clone()),
                    None => (window.title.clone(), window.editor.clone()),
                }
            }
            List::Actions => {
                let pinned = self.actions_project().is_some_and(|p| p.pinned);
                let (title, subtitle) = match PROJECT_ACTIONS[ix] {
                    ProjectAction::OpenWith => (
                        "Open with…",
                        "Another editor just this once, or set its default",
                    ),
                    ProjectAction::ShowInFileManager => {
                        (super::shortcuts::file_manager().0, "The project's folder")
                    }
                    ProjectAction::Terminal => (
                        "Open a terminal there",
                        "Windows Terminal if it's installed",
                    ),
                    ProjectAction::TogglePin if pinned => ("Unpin", "Back into the recent order"),
                    ProjectAction::TogglePin => ("Pin", "Pinned projects stay on top"),
                    ProjectAction::Rename => (
                        "Rename…",
                        "A shorter name; search still finds it by its folder",
                    ),
                    ProjectAction::CopyPath => ("Copy the path", "To the clipboard"),
                    ProjectAction::RepoPage => (
                        "Open the repository page",
                        "From the git origin remote (GitHub, GitLab…)",
                    ),
                    ProjectAction::Remove => {
                        ("Remove from the list", "The folder itself isn't touched")
                    }
                };
                (title.into(), subtitle.into())
            }
            List::Rename => Default::default(),
        }
    }

    /// Right-hand details: branch, own editor and last opened for projects;
    /// which editor is the default in the editor lists.
    pub(super) fn item_meta(&self, ix: usize, now: u64) -> Meta {
        match self.list() {
            List::Projects => {
                let project = &self.projects[ix];
                // Only projects with their own default name it; the rest use the global one.
                let editor = project.editor.as_deref().map(|c| self.name_of(c));
                let opened =
                    (project.last_opened > 0).then(|| store::ago(project.last_opened, now));
                let bottom: Vec<String> = editor.into_iter().chain(opened).collect();
                Meta {
                    top: project.branch.clone().map(|b| (b, self.theme.branch)),
                    bottom: (!bottom.is_empty()).then(|| bottom.join(" · ")),
                }
            }
            List::OpenWith => {
                let EditorOption::Detected(editor) = &self.editors[ix] else {
                    return Meta {
                        top: None,
                        bottom: None,
                    };
                };
                let own = self.open_with_project().and_then(|p| p.editor.as_deref());
                let is_global = self.config.editor.as_deref() == Some(editor.command.as_str());
                let role = if own == Some(editor.command.as_str()) {
                    Some("this project's default")
                } else if is_global && own.is_none() {
                    Some("default")
                } else if is_global {
                    Some("default for all projects")
                } else {
                    None
                };
                Meta {
                    top: role.map(|r| (r.to_string(), self.theme.accent)),
                    bottom: None,
                }
            }
            List::Editors => {
                let current = matches!(&self.editors[ix], EditorOption::Detected(e)
                    if self.config.editor.as_deref() == Some(e.command.as_str()));
                Meta {
                    top: current.then(|| ("default".to_string(), self.theme.accent)),
                    bottom: None,
                }
            }
            // Folders get a hint that → goes inside.
            List::Browse => Meta {
                top: self
                    .browse
                    .as_ref()
                    .filter(|b| b.entries[ix].is_dir)
                    .map(|_| ("→".to_string(), self.theme.muted)),
                bottom: None,
            },
            List::Switch => {
                let project = self.window_projects[ix].map(|p| &self.projects[p]);
                Meta {
                    top: project
                        .and_then(|p| p.branch.clone())
                        .map(|b| (b, self.theme.branch)),
                    bottom: project.map(|_| self.windows[ix].editor.clone()),
                }
            }
            // The action's own shortcut, where it has one.
            List::Actions => {
                let m = secondary();
                let key = match PROJECT_ACTIONS[ix] {
                    ProjectAction::OpenWith => Some("alt-↵".to_string()),
                    ProjectAction::ShowInFileManager => Some(format!("{m}-e")),
                    ProjectAction::Terminal => Some(format!("{m}-t")),
                    ProjectAction::CopyPath => Some(format!("{m}-c")),
                    ProjectAction::RepoPage => Some(format!("{m}-g")),
                    ProjectAction::TogglePin | ProjectAction::Rename | ProjectAction::Remove => {
                        None
                    }
                };
                Meta {
                    top: key.map(|k| (k, self.theme.muted)),
                    bottom: None,
                }
            }
            List::Commands | List::Rename => Meta {
                top: None,
                bottom: None,
            },
        }
    }

    /// The update command's title and subtitle, in step with the tray item.
    fn update_text(&self) -> (String, String) {
        let current = update::CURRENT;
        match &self.update {
            UpdateState::Unchecked => (
                "Check for updates".into(),
                format!("You have proj {current}"),
            ),
            UpdateState::Checking => (
                "Checking for updates…".into(),
                format!("You have proj {current}"),
            ),
            UpdateState::UpToDate => (
                "Check for updates".into(),
                format!("proj {current} is up to date"),
            ),
            UpdateState::Available(found) => (
                format!("Install update {}", found.version),
                format!("You have {current}; proj restarts on the new version"),
            ),
            UpdateState::Installing => (
                "Downloading update…".into(),
                "proj restarts when it's installed".into(),
            ),
            UpdateState::Failed => (
                "Check for updates".into(),
                "The last try failed; ↵ to try again".into(),
            ),
        }
    }

    pub(super) fn item_count(&self) -> usize {
        match self.list() {
            List::Projects => self.projects.len(),
            List::Browse => self.browse.as_ref().map_or(0, |b| b.entries.len()),
            List::Editors | List::OpenWith => self.editors.len(),
            List::Commands => self.commands.len(),
            List::Switch => self.windows.len(),
            List::Actions => PROJECT_ACTIONS.len(),
            List::Rename => 0,
        }
    }
}
