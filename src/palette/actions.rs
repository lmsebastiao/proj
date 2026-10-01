//! A project's actions: the ctrl-k menu (with its commands to run), which
//! opens over the list with a search box of its own, and the icons on the
//! highlighted row.

use gpui::{Context, Window};

use crate::{
    fuzzy, git, open,
    store::{self, Project},
    tasks,
    templates::Template,
};

use super::{
    Palette,
    items::{List, Match, Mode, ProjectAction, Target},
    keymap::{CopyPath, OpenRemote},
    theme::icons,
};

/// The icons on a project row. The rest (rename, remove…) are keys and menu
/// entries: an icon there was one stray click from opening the project.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum RowIcon {
    Pin,
    /// Opens the actions menu.
    More,
}

/// Left to right. Rows that aren't highlighted still show a pinned
/// project's pin.
pub(super) const ROW_ICONS: [RowIcon; 2] = [RowIcon::More, RowIcon::Pin];

impl RowIcon {
    /// For the icon's element id, with the row number.
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Pin => "pin",
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
    /// The match at this position in `Palette::menu_matches`.
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
    /// The actions menu is open over the project list.
    pub(super) fn menu_open(&self) -> bool {
        self.actions_for.is_some()
    }

    /// Ctrl-K: opens the actions menu for the selected project, or closes it
    /// when it's open.
    pub(super) fn show_actions(&mut self, cx: &mut Context<Self>) {
        if self.menu_open() {
            return self.close_menu(cx);
        }
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        self.show_shortcuts = false;
        self.numbers_shown = false;
        self.actions_for = Some(project.key());
        self.load_actions(&project);
        self.menu_query.clear();
        let placeholder = format!("Actions for {}…", project.name);
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
        self.load_actions(&project);
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
                    return self.problem(
                        format!(
                            "proj doesn't know what {host} runs. In the config file, add e.g. \
                             forges = {{ \"{host}\" = \"gitlab\" }} (or \"gitea\", \"forgejo\")"
                        ),
                        cx,
                    );
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

    /// A click on one of a project row's icons.
    pub(super) fn click_row_icon(&mut self, row: usize, icon: RowIcon, cx: &mut Context<Self>) {
        if self.list() != List::Projects {
            return;
        }
        self.close_menu(cx);
        self.selected = row;
        match icon {
            RowIcon::Pin => self.toggle_pin(cx),
            RowIcon::More => self.show_actions(cx),
        }
    }
}
