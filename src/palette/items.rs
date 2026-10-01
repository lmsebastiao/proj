//! What the list shows: item types, and each row's title, subtitle and details.

use std::path::PathBuf;

use crate::{
    config::{self, ThemeSetting},
    editors::Editor,
    git::{self, Forge},
    launcher::UpdateState,
    paths,
    store::{self, Project},
    update,
};

use super::{Palette, actions::Section, secondary};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Mode {
    Projects,
    /// Choosing the global editor.
    Editors,
    /// Choosing an editor for one project (`Palette::open_with`).
    OpenWith,
    /// Inside a project's folders (`Palette::browse`).
    Browse,
    /// Typing a new name for an entry (`Palette::editing`).
    Rename,
    /// Typing an entry's tags (`Palette::editing`).
    Tags,
    /// Typing a command for an entry's actions menu (`Palette::editing`).
    AddCommand,
    /// Choosing a template for a new project (`Palette::templates`).
    Templates,
    /// Typing the name of a new project made from `Palette::new_from`.
    NewProject,
    /// The window switcher: open editor windows (`Palette::windows`).
    Switch,
    /// Choosing what a self-hosted git site runs (`Palette::forge_pick`).
    Forges,
}

/// What the list currently shows. Commands appear when the query starts with `>`.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum List {
    Projects,
    Editors,
    OpenWith,
    Browse,
    Commands,
    /// Nothing: the search box holds what's being typed (a name, tags, a command).
    Text,
    /// The switcher's rows (`Palette::switch_rows`).
    Switch,
    Templates,
    /// What a git site can run (`forges::CHOICES`).
    Forges,
}

/// An entry in a project's actions menu (ctrl-k), as `Palette::actions` lists them.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ProjectAction {
    OpenWith,
    ShowInFileManager,
    Terminal,
    /// Runs `Palette::tasks[i]` in a terminal in the project's folder.
    Run(usize),
    AddCommand,
    TogglePin,
    Rename,
    Tags,
    CopyPath,
    RepoPage,
    PullRequests,
    Ci,
    CopyCloneUrl,
    NewFromThis,
    Remove,
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

#[derive(Clone, Copy, PartialEq)]
pub(super) enum PaletteCommand {
    Autostart,
    AddProjects,
    NewFromTemplate,
    ChangeEditor,
    /// Go through system → light → dark.
    Theme,
    OpenConfig,
    /// Forget the added projects whose folders are gone.
    RemoveMissing,
    /// Check for an update, or install the one found.
    Update,
    Quit,
}

/// The `>` commands. `updates`: this copy can update itself (it was
/// installed). `missing`: some projects' folders are gone.
pub(super) fn commands(updates: bool, missing: bool) -> Vec<PaletteCommand> {
    [
        PaletteCommand::Autostart,
        PaletteCommand::AddProjects,
        PaletteCommand::NewFromTemplate,
        PaletteCommand::ChangeEditor,
        PaletteCommand::Theme,
        PaletteCommand::OpenConfig,
    ]
    .into_iter()
    .chain(missing.then_some(PaletteCommand::RemoveMissing))
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
    /// What it means, shown when the mouse rests on it.
    pub(super) tip: Option<String>,
}

impl Meta {
    const NONE: Self = Self {
        top: None,
        bottom: None,
        tip: None,
    };
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
            List::Commands => self.command_text(self.commands[ix]),
            // The project's name over the window's title ("proj — main.rs"), or
            // over all its windows' titles; windows of no known project show
            // just their title. A project's own windows, once → opens them up,
            // show their titles.
            List::Switch => {
                let row = &self.switch_rows[ix];
                let window = &self.windows[row[0]];
                match self.window_projects[row[0]] {
                    _ if self.expanded.is_some() => (window.title.clone(), window.editor.clone()),
                    Some(project) if row.len() > 1 => {
                        let titles: Vec<&str> = row
                            .iter()
                            .map(|&w| self.windows[w].title.as_str())
                            .collect();
                        (
                            self.projects[project].name.clone(),
                            format!("{} windows · {}", row.len(), titles.join(" · ")),
                        )
                    }
                    Some(project) => (self.projects[project].name.clone(), window.title.clone()),
                    None => (window.title.clone(), window.editor.clone()),
                }
            }
            List::Templates => {
                let template = &self.templates[ix];
                (template.name(), template.source())
            }
            List::Forges => {
                let (label, _, what) = super::forges::CHOICES[ix];
                (label.into(), what.into())
            }
            List::Text => Default::default(),
        }
    }

    fn command_text(&self, command: PaletteCommand) -> (String, String) {
        let m = secondary();
        match command {
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
            PaletteCommand::NewFromTemplate => (
                "New project from a template…".into(),
                match self.config.templates.len() {
                    0 => "Set up templates in the config file".into(),
                    1 => "From the template in the config file".into(),
                    n => format!("From one of the {n} templates in the config file"),
                },
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
            PaletteCommand::RemoveMissing => {
                let missing = self.projects.iter().filter(|p| p.missing).count();
                let projects = if missing == 1 { "project" } else { "projects" };
                (
                    "Remove missing projects".into(),
                    format!("Forget the {missing} {projects} whose folders are gone"),
                )
            }
            PaletteCommand::Update => self.update_text(),
            PaletteCommand::Quit => (
                "Quit proj".into(),
                format!("Stop the background launcher · {m}-q"),
            ),
        }
    }

    pub(super) fn action_text(&self, action: ProjectAction) -> (String, String) {
        let project = self.actions_project();
        let pinned = project.is_some_and(|p| p.pinned);
        let (title, subtitle): (String, String) = match action {
            ProjectAction::OpenWith => (
                "Open with…".into(),
                "Another editor just this once, or set its default".into(),
            ),
            ProjectAction::ShowInFileManager => (
                super::shortcuts::file_manager().0.into(),
                "The project's folder".into(),
            ),
            ProjectAction::Terminal => (
                "Open a terminal there".into(),
                "Windows Terminal if it's installed".into(),
            ),
            ProjectAction::Run(i) => {
                let task = &self.tasks[i];
                let from = if task.added {
                    "Runs in a terminal there · shift-del removes it"
                } else {
                    "A package.json script, run in a terminal there"
                };
                (task.command.clone(), from.into())
            }
            ProjectAction::AddCommand => (
                "Add a command…".into(),
                "To run in a terminal there from this menu, e.g. npm run dev".into(),
            ),
            ProjectAction::TogglePin if pinned => {
                ("Unpin".into(), "Back into the recent order".into())
            }
            ProjectAction::TogglePin => ("Pin".into(), "Pinned projects stay on top".into()),
            ProjectAction::Rename => (
                "Rename…".into(),
                "A shorter name; search still finds it by its folder".into(),
            ),
            // Its tags show on the right (see `item_meta`).
            ProjectAction::Tags => (
                "Tags…".into(),
                "Group projects, then search for them with #tag".into(),
            ),
            ProjectAction::CopyPath => ("Copy the path".into(), "To the clipboard".into()),
            ProjectAction::RepoPage => (
                "Open the repository page".into(),
                "From the git origin remote (GitHub, GitLab…)".into(),
            ),
            ProjectAction::PullRequests if self.actions_forge() == Some(Forge::GitLab) => (
                "Open the merge requests".into(),
                self.forge_hint("Its merge requests on GitLab"),
            ),
            ProjectAction::PullRequests => (
                "Open the pull requests".into(),
                self.forge_hint("Its pull requests on the repository's site"),
            ),
            ProjectAction::Ci => (
                "Open the CI runs".into(),
                self.forge_hint("Actions, pipelines or builds"),
            ),
            ProjectAction::CopyCloneUrl => (
                "Copy the clone URL".into(),
                project
                    .and_then(|p| git::git_remote_url(&p.path))
                    .unwrap_or_default(),
            ),
            ProjectAction::NewFromThis => (
                "New project from this one…".into(),
                "A copy without what git ignores, with a history of its own".into(),
            ),
            ProjectAction::Remove => (
                "Remove from the list".into(),
                "The folder itself isn't touched".into(),
            ),
        };
        (title, subtitle)
    }

    /// The pull request and CI actions' subtitle: `known` when the site is,
    /// else how to tell proj what it runs.
    fn forge_hint(&self, known: &str) -> String {
        match self.actions_forge() {
            Some(_) => known.to_string(),
            None => {
                let host = self
                    .actions_web_url()
                    .as_deref()
                    .and_then(git::url_host)
                    .map(str::to_string)
                    .unwrap_or_default();
                format!("↵ asks what {host} runs (GitLab, Gitea…), just the first time")
            }
        }
    }

    /// The heading over a section of the actions menu; none over `Danger`,
    /// which gets a line instead.
    pub(super) fn section_label(&self, section: Section) -> Option<String> {
        Some(match section {
            Section::Open => "Open".into(),
            Section::Run => "Run".into(),
            Section::Organize => "Organize".into(),
            Section::Repository => {
                let host = self
                    .actions_web_url()
                    .as_deref()
                    .and_then(git::url_host)
                    .map(str::to_string);
                let site = self.actions_forge().map_or("Repository", Forge::name);
                match host {
                    Some(host) => format!("{site} · {host}"),
                    None => site.into(),
                }
            }
            Section::More => "More".into(),
            Section::Danger => return None,
        })
    }

    /// The web page of the actions menu's project, and the kind of site it's on.
    pub(super) fn actions_web_url(&self) -> Option<String> {
        git::git_web_url(&self.actions_project()?.path)
    }

    pub(super) fn actions_forge(&self) -> Option<Forge> {
        Forge::for_url(&self.actions_web_url()?, &self.config.forges)
    }

    /// The branch, with what `git status` said about it: "main ● ↑2". A long
    /// name loses its middle, where branch names tend to be alike
    /// ("feature/…-login-fix").
    pub(super) fn branch_label(&self, project: &Project) -> Option<String> {
        let branch = middle_ellipsis(project.branch.as_ref()?, 32);
        let status = self
            .git_status
            .get(&project.path)
            .map(|s| s.suffix())
            .unwrap_or_default();
        Some(format!("{branch}{status}"))
    }

    /// What the marks after a branch mean, when it has any.
    fn branch_tip(&self, project: &Project) -> Option<String> {
        let status = self.git_status.get(&project.path)?;
        let commits = |n: u32| if n == 1 { "commit" } else { "commits" };
        let mut parts = Vec::new();
        if status.dirty {
            parts.push("● uncommitted changes".to_string());
        }
        if status.ahead > 0 {
            parts.push(format!(
                "↑{} {} to push",
                status.ahead,
                commits(status.ahead)
            ));
        }
        if status.behind > 0 {
            parts.push(format!(
                "↓{} {} to pull (as of the last fetch)",
                status.behind,
                commits(status.behind)
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Right-hand details: branch, own editor and last opened for projects;
    /// which editor is the default in the editor lists.
    pub(super) fn item_meta(&self, ix: usize, now: u64) -> Meta {
        match self.list() {
            List::Projects => {
                let project = &self.projects[ix];
                if project.missing {
                    let gone = project
                        .paths()
                        .into_iter()
                        .find(|p| !p.is_dir())
                        .unwrap_or_else(|| project.path.clone());
                    return Meta {
                        top: Some(("missing".into(), self.theme.danger)),
                        bottom: Some("its folder is gone".into()),
                        tip: Some(format!(
                            "{} isn't there: moved, deleted, or on a drive that isn't \
                             connected. Remove it with {}-k, or every missing one with > \
                             Remove missing projects.",
                            paths::display_path(&gone),
                            secondary()
                        )),
                    };
                }
                // Only projects with their own default name it; the rest use the global one.
                let editor = project.editor.as_deref().map(|c| self.name_of(c));
                let opened =
                    (project.last_opened > 0).then(|| store::ago(project.last_opened, now));
                let bottom: Vec<String> = editor.into_iter().chain(opened).collect();
                Meta {
                    top: self.branch_label(project).map(|b| (b, self.theme.branch)),
                    bottom: (!bottom.is_empty()).then(|| bottom.join(" · ")),
                    // The marks after the branch, and when exactly "3d ago" was.
                    tip: {
                        let opened = (project.last_opened > 0)
                            .then(|| format!("Opened {}", store::date_of(project.last_opened)));
                        let lines: Vec<String> =
                            self.branch_tip(project).into_iter().chain(opened).collect();
                        (!lines.is_empty()).then(|| lines.join("\n"))
                    },
                }
            }
            List::OpenWith => {
                let EditorOption::Detected(editor) = &self.editors[ix] else {
                    return Meta::NONE;
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
                    tip: None,
                }
            }
            List::Editors => {
                let current = matches!(&self.editors[ix], EditorOption::Detected(e)
                    if self.config.editor.as_deref() == Some(e.command.as_str()));
                Meta {
                    top: current.then(|| ("default".to_string(), self.theme.accent)),
                    bottom: None,
                    tip: None,
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
                tip: None,
            },
            List::Switch => {
                let row = &self.switch_rows[ix];
                let project = self.window_projects[row[0]].map(|p| &self.projects[p]);
                let mut editors: Vec<&str> = Vec::new();
                for &w in row {
                    if !editors.contains(&self.windows[w].editor.as_str()) {
                        editors.push(&self.windows[w].editor);
                    }
                }
                let expanded = self.expanded.is_some();
                Meta {
                    top: project
                        .and_then(|p| self.branch_label(p))
                        .map(|b| (b, self.theme.branch)),
                    // A project's several windows: → shows them.
                    bottom: (project.is_some() && !expanded).then(|| {
                        let more = if row.len() > 1 { " · →" } else { "" };
                        format!("{}{more}", editors.join(", "))
                    }),
                    tip: match project {
                        Some(p) if row.len() > 1 && !expanded => Some(format!(
                            "{} windows of {}: ↵ switches to the one you used last, → lists \
                             them",
                            row.len(),
                            p.name
                        )),
                        Some(p) => self.branch_tip(p),
                        None => None,
                    },
                }
            }
            List::Templates => Meta {
                top: Some((
                    match self.templates[ix] {
                        crate::templates::Template::Folder(_) => "folder".into(),
                        crate::templates::Template::Git(_) => "git".into(),
                    },
                    self.theme.muted,
                )),
                bottom: None,
                tip: None,
            },
            List::Commands | List::Text | List::Forges => Meta::NONE,
        }
    }

    /// What shows on the right of an action in the menu: its own shortcut,
    /// where it has one, or the project's tags.
    pub(super) fn action_detail(&self, action: ProjectAction) -> Option<String> {
        let m = secondary();
        match action {
            ProjectAction::OpenWith => Some(format!("{m}-↵")),
            ProjectAction::ShowInFileManager => Some(format!("{m}-e")),
            ProjectAction::Terminal => Some(format!("{m}-t")),
            ProjectAction::CopyPath => Some(format!("{m}-c")),
            ProjectAction::RepoPage => Some(format!("{m}-g")),
            ProjectAction::TogglePin => Some(format!("{m}-shift-p")),
            ProjectAction::Rename => Some("f2".into()),
            ProjectAction::Remove => Some("shift-del".into()),
            ProjectAction::Tags => self
                .actions_project()
                .filter(|p| !p.tags.is_empty())
                .map(|p| {
                    let tags: Vec<String> = p.tags.iter().map(|t| format!("#{t}")).collect();
                    tags.join(" ")
                }),
            _ => None,
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
            List::Switch => self.switch_rows.len(),
            List::Templates => self.templates.len(),
            List::Forges => super::forges::CHOICES.len(),
            List::Text => 0,
        }
    }
}

/// `text` cut to `max` characters by taking out its middle: "feature/…-login-fix".
fn middle_ellipsis(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    // More of the end, where the part that tells branches apart usually is.
    let head = (max - 1) * 2 / 5;
    let tail = max - 1 - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_names_lose_their_middle() {
        assert_eq!(middle_ellipsis("main", 32), "main");
        let cut = middle_ellipsis("feature/PROJ-1234-rework-the-login-page-fix", 32);
        assert_eq!(cut.chars().count(), 32);
        assert_eq!(cut, "feature/PROJ…-the-login-page-fix");
    }
}
