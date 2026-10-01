//! A project's actions: the ctrl-k menu (with its commands to run), and the
//! icons on the highlighted row.

use gpui::{ClipboardItem, Context, Window};

use crate::{
    git, open,
    store::{self, Project},
    tasks,
    templates::Template,
};

use super::{
    Palette,
    items::{List, Mode, ProjectAction, Target},
    keymap::{CopyPath, OpenRemote, RemoveItem},
    theme::icons,
};

/// The icons on a project row.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum RowIcon {
    Pin,
    Rename,
    Remove,
    /// Opens the actions menu.
    More,
}

/// Left to right. Any order works: rows that aren't highlighted still show a
/// pinned project's pin, at the end.
pub(super) const ROW_ICONS: [RowIcon; 4] = [
    RowIcon::Rename,
    RowIcon::Remove,
    RowIcon::More,
    RowIcon::Pin,
];

impl RowIcon {
    /// For the icon's element id, with the row number.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Pin => "pin",
            Self::Rename => "rename",
            Self::Remove => "remove",
            Self::More => "more",
        }
    }
}

/// The actions menu's groups, in their order; each but `Danger` under a heading.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Section {
    Open,
    Run,
    Organize,
    /// Named after the repository's site, e.g. "GitLab · git.example.com".
    Repository,
    More,
    /// Remove, on its own under a line.
    Danger,
}

/// What the actions menu draws, top to bottom.
#[derive(Clone, PartialEq)]
pub(super) enum ActionEntry {
    Heading(String),
    /// Over the `Danger` section.
    Line,
    /// The match at this position in `Palette::matches`.
    Row(usize),
}

impl ProjectAction {
    pub(super) fn section(self) -> Section {
        match self {
            Self::OpenWith | Self::ShowInFileManager | Self::Terminal => Section::Open,
            Self::Run(_) | Self::AddCommand => Section::Run,
            Self::TogglePin | Self::Rename | Self::Tags => Section::Organize,
            Self::RepoPage | Self::PullRequests | Self::Ci | Self::CopyCloneUrl => {
                Section::Repository
            }
            Self::CopyPath | Self::NewFromThis => Section::More,
            Self::Remove => Section::Danger,
        }
    }

    /// Its glyph in the icon font. `pinned`: the project is.
    pub(super) fn icon(self, pinned: bool) -> &'static str {
        match self {
            Self::OpenWith => icons::OPEN_WITH,
            Self::ShowInFileManager => icons::FOLDER,
            Self::Terminal => icons::TERMINAL,
            Self::Run(_) => icons::RUN,
            Self::AddCommand => icons::ADD,
            Self::TogglePin if pinned => icons::PINNED,
            Self::TogglePin => icons::PIN,
            Self::Rename => icons::RENAME,
            Self::Tags => icons::TAG,
            Self::RepoPage => icons::GLOBE,
            Self::PullRequests => icons::PULL_REQUESTS,
            Self::Ci => icons::CI,
            Self::CopyCloneUrl => icons::LINK,
            Self::CopyPath => icons::COPY,
            Self::NewFromThis => icons::NEW_PROJECT,
            Self::Remove => icons::REMOVE,
        }
    }
}

impl Palette {
    /// Ctrl-K: the actions menu for the selected project.
    pub(super) fn show_actions(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        self.actions_for = Some(project.key());
        self.load_actions(&project);
        self.set_mode(Mode::Actions, cx);
    }

    /// The menu's entries for `project`, section by section (see `Section`).
    /// One whose folder is gone gets the ones that don't need it.
    fn load_actions(&mut self, project: &Project) {
        use ProjectAction as A;
        if project.missing {
            self.tasks = Vec::new();
            self.actions = vec![A::TogglePin, A::Rename, A::Tags, A::CopyPath, A::Remove];
            return;
        }
        self.tasks = tasks::tasks(project, &self.db);
        let remote = git::git_remote_url(&project.path).is_some();
        let web = git::git_web_url(&project.path).is_some();
        let mut actions = vec![A::OpenWith, A::ShowInFileManager, A::Terminal];
        actions.extend((0..self.tasks.len()).map(A::Run));
        actions.extend([A::AddCommand, A::TogglePin, A::Rename, A::Tags]);
        if web {
            actions.extend([A::RepoPage, A::PullRequests, A::Ci]);
        }
        if remote {
            actions.push(A::CopyCloneUrl);
        }
        actions.push(A::CopyPath);
        if !project.is_workspace() {
            actions.push(A::NewFromThis);
        }
        actions.push(A::Remove);
        self.actions = actions;
    }

    /// Shift-Delete on a command added to the actions menu: takes it out again.
    pub(super) fn remove_task(&mut self, _: &RemoveItem, _: &mut Window, cx: &mut Context<Self>) {
        if self.list() != List::Actions {
            return;
        }
        let Some(ProjectAction::Run(i)) =
            self.matches.get(self.selected).map(|m| self.actions[m.ix])
        else {
            return;
        };
        let (Some(key), Some(task)) = (self.actions_for.clone(), self.tasks.get(i).cloned()) else {
            return;
        };
        if !task.added {
            self.status = Some("That one is from package.json".into());
            cx.notify();
            return;
        }
        store::remove_command(&mut self.db, &key, &task.command);
        self.save(cx);
        let Some(project) = self.actions_project().cloned() else {
            return;
        };
        let selected = self.selected;
        self.load_actions(&project);
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.notice(format!("Removed \"{}\"", task.command), cx);
    }

    /// The menu's rows under their section headings; while searching, just
    /// the matches, best first.
    pub(super) fn action_entries(&self) -> Vec<ActionEntry> {
        if !self.filter_query().is_empty() {
            return (0..self.matches.len()).map(ActionEntry::Row).collect();
        }
        let mut entries = Vec::new();
        let mut section = None;
        for (row, m) in self.matches.iter().enumerate() {
            let this = self.actions[m.ix].section();
            if section != Some(this) {
                entries.push(match self.section_label(this) {
                    Some(label) => ActionEntry::Heading(label),
                    None => ActionEntry::Line,
                });
                section = Some(this);
            }
            entries.push(ActionEntry::Row(row));
        }
        entries
    }

    pub(super) fn actions_project(&self) -> Option<&Project> {
        let key = self.actions_for.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    /// Runs a menu action. The actions work on the selected project, so this
    /// goes back to the project list with it selected first.
    pub(super) fn run_action(
        &mut self,
        action: ProjectAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.actions_for.clone() else {
            return;
        };
        // Worked out while the menu's project is still known.
        let web = self.actions_web_url();
        let forge = self.actions_forge();
        self.back_to_projects(cx);
        let Some(project) = self.selected_project().filter(|p| p.key() == key).cloned() else {
            return;
        };
        match action {
            ProjectAction::OpenWith => self.show_open_with(key, cx),
            ProjectAction::ShowInFileManager => {
                self.open_selected(Target::FileManager, window, cx);
            }
            ProjectAction::Terminal => self.open_selected(Target::Terminal, window, cx),
            ProjectAction::Run(i) => {
                let Some(task) = self.tasks.get(i).cloned() else {
                    return;
                };
                match open::run_in_terminal(&project.path, &task.command) {
                    Ok(()) => window.remove_window(),
                    Err(err) => {
                        self.status = Some(format!("Could not run {}: {err}", task.command).into());
                        cx.notify();
                    }
                }
            }
            ProjectAction::AddCommand => self.edit_selected(Mode::AddCommand, cx),
            ProjectAction::TogglePin => self.toggle_pin(cx),
            ProjectAction::Rename => self.rename(cx),
            ProjectAction::Tags => self.edit_selected(Mode::Tags, cx),
            ProjectAction::CopyPath => self.copy_path(&CopyPath, window, cx),
            ProjectAction::RepoPage => self.open_remote(&OpenRemote, window, cx),
            ProjectAction::PullRequests | ProjectAction::Ci => {
                let Some(web) = web else {
                    return;
                };
                let Some(forge) = forge else {
                    let host = git::url_host(&web).unwrap_or_default();
                    self.status = Some(
                        format!(
                            "proj doesn't know what {host} runs. In the config file, add e.g. \
                             forges = {{ \"{host}\" = \"gitlab\" }} (or \"gitea\", \"forgejo\")"
                        )
                        .into(),
                    );
                    cx.notify();
                    return;
                };
                let url = if action == ProjectAction::Ci {
                    forge.ci(&web)
                } else {
                    forge.pull_requests(&web)
                };
                match open::open_url(&url) {
                    Ok(()) => window.remove_window(),
                    Err(err) => {
                        self.status = Some(format!("Could not open {url}: {err}").into());
                        cx.notify();
                    }
                }
            }
            ProjectAction::CopyCloneUrl => {
                if let Some(url) = git::git_remote_url(&project.path) {
                    cx.write_to_clipboard(ClipboardItem::new_string(url));
                    window.remove_window();
                }
            }
            ProjectAction::NewFromThis => {
                self.new_from = Some(Template::Folder(project.path.clone()));
                self.set_mode(Mode::NewProject, cx);
            }
            ProjectAction::Remove => self.remove(cx),
        }
    }

    /// A click on one of a project row's icons. Remove asks for a second click,
    /// as a stray click is easier than a stray key.
    pub(super) fn click_row_icon(&mut self, row: usize, icon: RowIcon, cx: &mut Context<Self>) {
        if self.list() != List::Projects {
            return;
        }
        let key = self.matches.get(row).map(|m| self.projects[m.ix].key());
        if row != self.selected {
            self.selected = row;
            self.confirm_remove = None;
        }
        match icon {
            RowIcon::Pin => self.toggle_pin(cx),
            RowIcon::Rename => self.rename(cx),
            RowIcon::More => self.show_actions(cx),
            RowIcon::Remove if self.confirm_remove.is_some() && self.confirm_remove == key => {
                self.remove(cx);
            }
            RowIcon::Remove => {
                let name = self
                    .selected_project()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                self.status = Some(format!("Click the bin again to remove {name}").into());
                self.confirm_remove = key;
                cx.notify();
            }
        }
    }
}
