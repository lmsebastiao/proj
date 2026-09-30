//! What the list shows: item types, and each row's title, subtitle and details.

use std::path::PathBuf;

use crate::{config, editors::Editor, paths, store};

use super::{Palette, secondary, theme::*};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Projects,
    /// Choosing the global editor (`Palette::open_with` remembers the project
    /// that was selected, to return to it).
    Editors,
    /// Choosing an editor for one project (`Palette::open_with`).
    OpenWith,
    /// Inside a project's folders (`Palette::browse`).
    Browse,
    /// Typing a new name for an entry (`Palette::renaming`).
    Rename,
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
}

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

#[derive(Clone, Copy)]
pub(super) enum PaletteCommand {
    Autostart,
    AddProjects,
    ChangeEditor,
    OpenConfig,
    Quit,
}

pub(super) const COMMANDS: [PaletteCommand; 5] = [
    PaletteCommand::Autostart,
    PaletteCommand::AddProjects,
    PaletteCommand::ChangeEditor,
    PaletteCommand::OpenConfig,
    PaletteCommand::Quit,
];

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
                match COMMANDS[ix] {
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
                            "All projects open in {} unless they have their own · {m}-shift-e",
                            self.name_of(self.config.editor.as_deref().unwrap_or(""))
                        ),
                    ),
                    PaletteCommand::OpenConfig => (
                        "Open config file".into(),
                        paths::display_path(&config::config_path()),
                    ),
                    PaletteCommand::Quit => (
                        "Quit proj".into(),
                        format!("Stop the background launcher · {m}-q"),
                    ),
                }
            }
            List::Rename => Default::default(),
        }
    }

    /// Right-hand details: branch, editors and last opened for projects; which
    /// editor is the default in the editor lists.
    pub(super) fn item_meta(&self, ix: usize, now: u64) -> Meta {
        match self.list() {
            List::Projects => {
                let project = &self.projects[ix];
                let editors = (!project.editors.is_empty()).then(|| {
                    project
                        .editors
                        .iter()
                        .map(|c| self.name_of(c))
                        .collect::<Vec<_>>()
                        .join(" / ")
                });
                let opened =
                    (project.last_opened > 0).then(|| store::ago(project.last_opened, now));
                let bottom: Vec<String> = editors.into_iter().chain(opened).collect();
                Meta {
                    top: project.branch.clone().map(|b| (b, BRANCH)),
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
                let own = self
                    .open_with_project()
                    .map(|p| p.editors.as_slice())
                    .unwrap_or_default();
                let is_global = self.config.editor.as_deref() == Some(editor.command.as_str());
                let role = if own.first() == Some(&editor.command) {
                    Some("this project's default")
                } else if own.contains(&editor.command) {
                    Some("in this project's list")
                } else if is_global && own.is_empty() {
                    Some("default")
                } else if is_global {
                    Some("default for other projects")
                } else {
                    None
                };
                Meta {
                    top: role.map(|r| (r.to_string(), ACCENT)),
                    bottom: None,
                }
            }
            List::Editors => {
                let current = matches!(&self.editors[ix], EditorOption::Detected(e)
                    if self.config.editor.as_deref() == Some(e.command.as_str()));
                Meta {
                    top: current.then(|| ("default".to_string(), ACCENT)),
                    bottom: None,
                }
            }
            // Folders get a hint that → goes inside.
            List::Browse => Meta {
                top: self
                    .browse
                    .as_ref()
                    .filter(|b| b.entries[ix].is_dir)
                    .map(|_| ("→".to_string(), MUTED)),
                bottom: None,
            },
            List::Commands | List::Rename => Meta {
                top: None,
                bottom: None,
            },
        }
    }

    pub(super) fn item_count(&self) -> usize {
        match self.list() {
            List::Projects => self.projects.len(),
            List::Browse => self.browse.as_ref().map_or(0, |b| b.entries.len()),
            List::Editors | List::OpenWith => self.editors.len(),
            List::Commands => COMMANDS.len(),
            List::Rename => 0,
        }
    }
}
