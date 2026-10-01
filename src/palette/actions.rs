//! A project's menus, which open over the list with a search box of their
//! own: its actions (ctrl-k) and its commands to run (ctrl-r).

use gpui::{Context, Window};

use crate::{
    fuzzy, git, open,
    store::{self, Project},
    tasks,
    templates::Template,
};

use super::{
    Palette,
    forges::ForgePick,
    items::{List, Match, Mode, ProjectAction, Target},
    keymap::{CopyPath, OpenRemote},
    theme::icons,
};

/// Which of a project's menus is open.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum MenuKind {
    /// Ctrl-K: everything that can be done with it.
    Actions,
    /// Ctrl-R: its commands to run in a terminal.
    Commands,
}

/// The menus' groups, in their order; each but `Danger` and `AddNew` under a
/// heading.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Section {
    Open,
    Organize,
    /// Named after the repository's site, e.g. "GitLab · git.example.com".
    Repository,
    More,
    /// Remove, on its own under a line.
    Danger,
    /// The commands menu: the ones added by hand…
    Added,
    /// …then the package.json scripts…
    Scripts,
    /// …then "Add a command…", under a line.
    AddNew,
}

/// What the actions menu draws, top to bottom.
#[derive(Clone, PartialEq)]
pub(super) enum ActionEntry {
    Heading(String),
    /// Over the `Danger` section.
    Line,
    /// The match at this position in `Palette::menu_matches`.
    Row(usize),
}

impl ProjectAction {
    /// Its glyph in the icon font.
    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::OpenWith => icons::OPEN_WITH,
            Self::ShowInFileManager => icons::FOLDER,
            Self::Terminal => icons::TERMINAL,
            Self::Commands | Self::Run(_) => icons::RUN,
            Self::AddCommand => icons::ADD,
            Self::Rename => icons::RENAME,
            Self::Tags => icons::TAG,
            Self::RepoPage => icons::GLOBE,
            Self::PullRequests => icons::PULL_REQUESTS,
            Self::Ci => icons::CI,
            Self::ChangeForge => icons::SETTINGS,
            Self::CopyCloneUrl => icons::LINK,
            Self::CopyPath => icons::COPY,
            Self::NewFromThis => icons::NEW_PROJECT,
            Self::Remove => icons::REMOVE,
        }
    }
}

impl Palette {
    /// The actions menu is open over the project list.
    pub(super) fn menu_open(&self) -> bool {
        self.actions_for.is_some()
    }

    /// Ctrl-K: opens the actions menu for the selected project, or closes it
    /// when it's open.
    pub(super) fn show_actions(&mut self, cx: &mut Context<Self>) {
        self.show_menu(MenuKind::Actions, cx);
    }

    /// Ctrl-R: the same for its commands to run.
    pub(super) fn show_commands(&mut self, cx: &mut Context<Self>) {
        self.show_menu(MenuKind::Commands, cx);
    }

    /// Opens the `kind` menu over the list; its own key again closes it, the
    /// other one's switches to that.
    fn show_menu(&mut self, kind: MenuKind, cx: &mut Context<Self>) {
        if self.menu_open() && self.menu_kind == kind {
            return self.close_menu(cx);
        }
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        // Its commands run in its folder.
        if kind == MenuKind::Commands && self.say_if_missing(&project, cx) {
            return;
        }
        self.show_shortcuts = false;
        self.numbers_shown = false;
        self.actions_for = Some(project.key());
        self.menu_kind = kind;
        self.load_actions(&project, kind);
        self.menu_query.clear();
        let placeholder = match kind {
            MenuKind::Actions => format!("Actions for {}…", project.name),
            MenuKind::Commands => format!("Commands for {}…", project.name),
        };
        self.menu_input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, cx);
            input.set_text("", cx);
        });
        self.refilter_menu(cx);
    }

    /// Back to the list, with the same project highlighted. Focus follows in
    /// `render`.
    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        self.actions_for = None;
        cx.notify();
    }

    /// The `kind` menu's entries for `project`, section by section (see
    /// `Section`). One whose folder is gone gets the actions that don't need it.
    fn load_actions(&mut self, project: &Project, kind: MenuKind) {
        use ProjectAction as A;
        if kind == MenuKind::Commands {
            self.tasks = tasks::tasks(project, &self.db);
            let mut actions: Vec<ProjectAction> = (0..self.tasks.len()).map(A::Run).collect();
            actions.push(A::AddCommand);
            self.actions = actions;
            return;
        }
        self.tasks = Vec::new();
        if project.missing {
            self.actions = vec![A::Rename, A::Tags, A::CopyPath, A::Remove];
            return;
        }
        let remote = git::git_remote_url(&project.path).is_some();
        let web = git::git_web_url(&project.path).is_some();
        let mut actions = vec![A::OpenWith, A::ShowInFileManager, A::Terminal, A::Commands];
        actions.extend([A::Rename, A::Tags]);
        if web {
            actions.extend([A::RepoPage, A::PullRequests, A::Ci]);
        }
        if remote {
            actions.push(A::CopyCloneUrl);
        }
        if web {
            actions.push(A::ChangeForge);
        }
        actions.push(A::CopyPath);
        if !project.is_workspace() {
            actions.push(A::NewFromThis);
        }
        actions.push(A::Remove);
        self.actions = actions;
    }

    /// The group an entry of the open menu goes under.
    fn action_section(&self, action: ProjectAction) -> Section {
        use ProjectAction as A;
        match action {
            A::OpenWith | A::ShowInFileManager | A::Terminal | A::Commands => Section::Open,
            A::Run(i) if self.tasks.get(i).is_some_and(|t| t.added) => Section::Added,
            A::Run(_) => Section::Scripts,
            A::AddCommand => Section::AddNew,
            A::Rename | A::Tags => Section::Organize,
            A::RepoPage | A::PullRequests | A::Ci | A::CopyCloneUrl | A::ChangeForge => {
                Section::Repository
            }
            A::CopyPath | A::NewFromThis => Section::More,
            A::Remove => Section::Danger,
        }
    }

    /// Filters the menu by what's typed in its search box, best first.
    pub(super) fn refilter_menu(&mut self, cx: &mut Context<Self>) {
        let query = self.menu_query.clone();
        self.menu_matches = if query.is_empty() {
            (0..self.actions.len())
                .map(|ix| Match {
                    ix,
                    title_hl: Vec::new(),
                    subtitle_hl: Vec::new(),
                })
                .collect()
        } else {
            let mut scored: Vec<(i32, Match)> = (0..self.actions.len())
                .filter_map(|ix| {
                    let (title, subtitle) = self.action_text(self.actions[ix]);
                    let m = fuzzy::score_item(&query, &title, &subtitle, 0)?;
                    Some((
                        m.score,
                        Match {
                            ix,
                            title_hl: m.title_hl,
                            subtitle_hl: m.subtitle_hl,
                        },
                    ))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            scored.into_iter().map(|(_, m)| m).collect()
        };
        self.menu_selected = 0;
        self.actions_scroll.scroll_to_item(0);
        cx.notify();
    }

    /// ↑/↓ in the menu, going round at the ends.
    pub(super) fn select_in_menu(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.menu_matches.len();
        if len == 0 {
            return;
        }
        let row = if delta.abs() > 1 {
            // A page: stop at the ends.
            (self.menu_selected as isize + delta).clamp(0, len as isize - 1) as usize
        } else {
            (self.menu_selected as isize + delta).rem_euclid(len as isize) as usize
        };
        self.select_menu_row(row, cx);
    }

    pub(super) fn select_menu_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let len = self.menu_matches.len();
        if len == 0 {
            return;
        }
        self.menu_selected = row.min(len - 1);
        let entry = self
            .action_entries()
            .iter()
            .position(|e| *e == ActionEntry::Row(self.menu_selected));
        self.actions_scroll.scroll_to_item(entry.unwrap_or(0));
        cx.notify();
    }

    /// The menu's highlighted action.
    pub(super) fn selected_action(&self) -> Option<ProjectAction> {
        self.menu_matches
            .get(self.menu_selected)
            .map(|m| self.actions[m.ix])
    }

    /// Enter in the menu.
    pub(super) fn confirm_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = self.selected_action() {
            self.run_action(action, window, cx);
        }
    }

    /// Shift-Delete on a command added to the actions menu: takes it out again.
    pub(super) fn remove_task(&mut self, cx: &mut Context<Self>) {
        let Some(ProjectAction::Run(i)) = self.selected_action() else {
            return;
        };
        let (Some(key), Some(task)) = (self.actions_for.clone(), self.tasks.get(i).cloned()) else {
            return;
        };
        if !task.added {
            return self.problem("That one is from package.json", cx);
        }
        store::remove_command(&mut self.db, &key, &task.command);
        self.save(cx);
        let Some(project) = self.actions_project().cloned() else {
            return;
        };
        let selected = self.menu_selected;
        self.load_actions(&project, self.menu_kind);
        self.refilter_menu(cx);
        self.menu_selected = selected.min(self.menu_matches.len().saturating_sub(1));
        self.notice(format!("Removed \"{}\"", task.command), cx);
    }

    /// The menu's rows under their section headings; while searching, just
    /// the matches, best first.
    pub(super) fn action_entries(&self) -> Vec<ActionEntry> {
        if !self.menu_query.is_empty() {
            return (0..self.menu_matches.len()).map(ActionEntry::Row).collect();
        }
        let mut entries = Vec::new();
        let mut section = None;
        for (row, m) in self.menu_matches.iter().enumerate() {
            let this = self.action_section(self.actions[m.ix]);
            if section != Some(this) {
                match self.section_label(this) {
                    Some(label) => entries.push(ActionEntry::Heading(label)),
                    // A line only between groups: none over a menu that's
                    // just "Add a command…".
                    None if !entries.is_empty() => entries.push(ActionEntry::Line),
                    None => {}
                }
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

    /// Runs a menu action. The actions work on the selected project, which is
    /// still highlighted under the menu, so this closes the menu first.
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
        self.close_menu(cx);
        let Some(project) = self.selected_project().filter(|p| p.key() == key).cloned() else {
            return;
        };
        match action {
            ProjectAction::OpenWith => self.show_open_with(key, cx),
            ProjectAction::Commands => self.show_commands(cx),
            ProjectAction::ChangeForge => {
                let Some(web) = web else {
                    return;
                };
                let host = git::url_host(&web).unwrap_or_default().to_string();
                let pick = ForgePick {
                    web,
                    host,
                    key,
                    then: None,
                };
                self.ask_forge(pick, cx);
            }
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
                        self.problem(format!("Could not run {}: {err}", task.command), cx);
                    }
                }
            }
            ProjectAction::AddCommand => self.edit_selected(Mode::AddCommand, cx),
            ProjectAction::Rename => self.rename(cx),
            ProjectAction::Tags => self.edit_selected(Mode::Tags, cx),
            ProjectAction::CopyPath => self.copy_path(&CopyPath, window, cx),
            ProjectAction::RepoPage => self.open_remote(&OpenRemote, window, cx),
            ProjectAction::PullRequests | ProjectAction::Ci => {
                let Some(web) = web else {
                    return;
                };
                // A site proj can't tell from its name: ask what it runs.
                let Some(forge) = forge else {
                    let host = git::url_host(&web).unwrap_or_default().to_string();
                    let pick = ForgePick {
                        web,
                        host,
                        key,
                        then: Some(action),
                    };
                    return self.ask_forge(pick, cx);
                };
                let url = if action == ProjectAction::Ci {
                    forge.ci(&web)
                } else {
                    forge.pull_requests(&web)
                };
                match open::open_url(&url) {
                    Ok(()) => window.remove_window(),
                    Err(err) => self.problem(format!("Could not open {url}: {err}"), cx),
                }
            }
            ProjectAction::CopyCloneUrl => {
                if let Some(url) = git::git_remote_url(&project.path) {
                    self.copy_and_close(url, "the clone URL", window, cx);
                }
            }
            ProjectAction::NewFromThis => {
                self.new_from = Some(Template::Folder(project.path.clone()));
                self.set_mode(Mode::NewProject, cx);
            }
            ProjectAction::Remove => self.remove(cx),
        }
    }

    /// The ⋯ on a project row, or right-clicking it: that row's actions.
    pub(super) fn actions_for_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.list() != List::Projects {
            return;
        }
        self.close_menu(cx);
        self.selected = row;
        self.show_actions(cx);
    }
}
