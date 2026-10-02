//! The search dialog: state, modes, filtering and selection. Actions live in
//! `projects`, `editor_choice`, `browse` and `switch` (the window switcher),
//! drawing in `render`, and the list of keys in `shortcuts`.

mod actions;
mod app_icon;
mod browse;
mod editor_choice;
mod files;
mod forges;
mod groups;
mod items;
mod keymap;
mod projects;
mod render;
mod scrollbar;
mod shortcuts;
mod switch;
mod theme;
mod tooltip;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use git::GitStatus;

use global_hotkey::hotkey::Modifiers;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Global, ListAlignment, ListState, ScrollHandle,
    SharedString, Subscription, Window, prelude::*, px,
};

use crate::{
    autostart,
    config::{self, Config},
    editors::{self, Editor},
    fuzzy, git,
    input::{self, TextInput},
    launcher::{UpdateState, Updates},
    paths,
    store::{self, Db, Project},
    switcher::EditorWindow,
    tasks::Task,
    templates::Template,
    update,
};

use files::{FILES_PREFIX, FileSearch};
use items::{
    CloneTarget, EditorOption, List, Match, Mode, PaletteCommand, PastedPath, ProjectAction, Target,
};
use keymap::{Confirm, Dismiss};
use theme::{ROW_GAP, ROW_HEIGHT, Theme};

pub use keymap::bind_keys;

/// Shortcuts that couldn't be registered, shown in the footer each time the
/// palette opens until `hotkey` in config.toml is fixed.
pub struct ShortcutNotice(pub Option<SharedString>);

impl Global for ShortcutNotice {}

/// The search typed when the palette was last closed without opening
/// anything, and when: opening it again soon after brings it back.
#[derive(Default)]
struct LastSearch(Option<(String, Instant)>);

impl Global for LastSearch {}

/// How long a closed palette's search is kept.
const KEEP_SEARCH: Duration = Duration::from_secs(30);

/// The footer's message.
#[derive(Clone, PartialEq)]
pub(super) struct Status {
    pub(super) text: SharedString,
    /// Something went wrong or can't be done: shown in full, in the warning
    /// colour, above the footer.
    pub(super) problem: bool,
    /// Ctrl-z undoes what it says, so it gets an Undo button.
    pub(super) undo: bool,
}

pub struct Palette {
    input: Entity<TextInput>,
    mode: Mode,
    query: String,
    config: Config,
    db: Db,
    projects: Vec<Project>,
    editors: Vec<EditorOption>,
    /// Editor command -> display name.
    names: HashMap<String, String>,
    /// Projects' own editors' and the global editor's commands -> the
    /// program, for its icon.
    apps: HashMap<String, Option<PathBuf>>,
    /// Key of the entry being opened in `Mode::OpenWith`.
    open_with: Option<String>,
    /// The project being browsed in `Mode::Browse`.
    browse: Option<browse::Browse>,
    /// Key of the entry being renamed, tagged or given a command
    /// (`Mode::Rename`, `Mode::Tags`, `Mode::AddCommand`).
    editing: Option<String>,
    /// Key of the entry whose actions (ctrl-k) or commands (ctrl-r) menu is
    /// open over the list, and which of the two.
    actions_for: Option<String>,
    menu_kind: actions::MenuKind,
    /// That menu's entries, and the tasks its `Run` entries run.
    actions: Vec<ProjectAction>,
    tasks: Vec<Task>,
    /// The menu's own search box, what's typed in it, its matches (into
    /// `actions`) and its highlighted one.
    menu_input: Entity<TextInput>,
    menu_query: String,
    menu_matches: Vec<Match>,
    menu_selected: usize,
    /// `Mode::Templates`' list, from the config.
    templates: Vec<Template>,
    /// What `Mode::NewProject` makes the new project from.
    new_from: Option<Template>,
    /// The git site `Mode::Forges` asks about.
    forge_pick: Option<forges::ForgePick>,
    /// The "Open together" page, or a group's "Folders…" (`Mode::Group`).
    group_page: Option<groups::GroupPage>,
    /// Projects being opened together without being saved as a group: in
    /// `projects` too, until the list shows again.
    unsaved: Option<Project>,
    /// What `git status` said, by folder; filled in while the palette is open.
    git_status: HashMap<PathBuf, GitStatus>,
    /// Projects marked with tab, to open together as one workspace.
    marked: Vec<PathBuf>,
    /// `$`: the projects' files, as read so far, and what was found in them.
    files: FileSearch,
    autostart: bool,
    /// The colours in use, from `config.theme` and the system setting.
    theme: Theme,
    /// The `>` commands.
    commands: Vec<PaletteCommand>,
    /// Where updating is at, for the update command (kept in step by an observer).
    update: UpdateState,
    matches: Vec<Match>,
    selected: usize,
    /// Set when the query is a path to a file or a not-yet-listed folder.
    pasted: Option<PastedPath>,
    /// Set when the query is a git URL.
    clone_candidate: Option<CloneTarget>,
    /// A native file dialog is open; don't treat the lost focus as a dismissal.
    picking: bool,
    /// `git clone` (or making a project from a template) is running. The
    /// palette stays open so it can open the result.
    cloning: bool,
    /// The dropdown with every shortcut (F1) is open.
    show_shortcuts: bool,
    /// The dropdown's highlighted row, which the arrows move while it's open.
    shortcut_selected: usize,
    shortcuts_scroll: ScrollHandle,
    /// The actions menu, drawn as a plain list for its section headings.
    actions_scroll: ScrollHandle,
    /// After copying: the palette is about to close.
    closing: bool,
    /// Open editor windows, for `Mode::Switch` and the dots of projects with one open.
    windows: Vec<EditorWindow>,
    /// The project each of `windows` shows, as an index into `projects`.
    window_projects: Vec<Option<usize>>,
    /// The switcher's rows, as indexes into `windows`: one per project (the
    /// window to switch to first), or each of `expanded`'s windows.
    switch_rows: Vec<Vec<usize>>,
    /// The project (index into `projects`) whose windows the switcher lists
    /// one by one, after → on its row.
    expanded: Option<usize>,
    /// Keys of the projects that have a window open.
    open_keys: HashSet<String>,
    /// The switcher's modifiers while they're held; letting go switches.
    hold: Option<Modifiers>,
    /// The footer's message: problems stay until the next key; `notice`s go
    /// by themselves.
    status: Option<Status>,
    /// The database before the last remove, and what was removed, for ctrl-z.
    /// Any other change to the database drops it.
    undo: Option<(Db, String)>,
    /// Ctrl (cmd on macOS) alone is down, and has been for a moment: the first
    /// nine projects show their number.
    ctrl_down: bool,
    numbers_shown: bool,
    list_state: ListState,
    /// The row the projects without a window open start at, under a line,
    /// as the list was last told its rows' heights.
    section: Option<usize>,
    /// Where on the scrollbar's thumb the mouse took hold of it.
    scrollbar_grab: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    /// The project search. `windows`: the open editor windows, in the switcher's order.
    pub fn new(window: &mut Window, cx: &mut Context<Self>, windows: Vec<EditorWindow>) -> Self {
        let input = cx.new(|cx| TextInput::new("", cx));
        let menu_input = cx.new(|cx| TextInput::new("", cx));
        let subscriptions = vec![
            cx.subscribe(&input, |this, input, _: &input::Changed, cx| {
                let query = input.read(cx).text().trim().to_string();
                if query != this.query {
                    this.query = query;
                    this.status = None;
                    this.show_shortcuts = false;
                    this.refilter(cx);
                }
            }),
            cx.subscribe(&menu_input, |this, input, _: &input::Changed, cx| {
                let query = input.read(cx).text().trim().to_string();
                if query != this.menu_query {
                    this.menu_query = query;
                    this.refilter_menu(cx);
                }
            }),
            // Behave like a launcher: clicking anywhere else dismisses it.
            cx.observe_window_activation(window, |this, window, cx| {
                if !this.picking && !this.cloning && !window.is_window_active() {
                    this.remember_search(cx);
                    window.remove_window();
                }
            }),
            // An update check or install moved on: redraw the update command.
            cx.observe_global::<Updates>(|this, cx| this.sync_update(cx)),
            // Windows switched between light and dark: follow it, if the theme does.
            cx.observe_window_appearance(window, |this, window, cx| {
                this.apply_theme(window, cx);
            }),
        ];
        let mut this = Self {
            input,
            mode: Mode::Projects,
            query: String::new(),
            config: Config::default(),
            db: Db::default(),
            projects: Vec::new(),
            editors: Vec::new(),
            names: HashMap::new(),
            apps: HashMap::new(),
            open_with: None,
            browse: None,
            editing: None,
            actions_for: None,
            menu_kind: actions::MenuKind::Actions,
            actions: Vec::new(),
            tasks: Vec::new(),
            menu_input,
            menu_query: String::new(),
            menu_matches: Vec::new(),
            menu_selected: 0,
            templates: Vec::new(),
            new_from: None,
            forge_pick: None,
            group_page: None,
            unsaved: None,
            git_status: HashMap::new(),
            marked: Vec::new(),
            files: FileSearch::default(),
            autostart: autostart::is_enabled(),
            theme: theme::DARK,
            commands: Vec::new(),
            update: cx
                .try_global::<Updates>()
                .map_or(UpdateState::Unchecked, |u| u.state.clone()),
            matches: Vec::new(),
            selected: 0,
            pasted: None,
            clone_candidate: None,
            picking: false,
            cloning: false,
            show_shortcuts: false,
            shortcut_selected: 0,
            shortcuts_scroll: ScrollHandle::new(),
            actions_scroll: ScrollHandle::new(),
            closing: false,
            windows: Vec::new(),
            window_projects: Vec::new(),
            switch_rows: Vec::new(),
            expanded: None,
            open_keys: HashSet::new(),
            hold: None,
            status: None,
            undo: None,
            ctrl_down: false,
            numbers_shown: false,
            // Every row measured, so wheel scrolling knows the list's full height.
            list_state: ListState::new(0, ListAlignment::Top, px(ROW_HEIGHT * 4.)).measure_all(),
            section: None,
            scrollbar_grab: None,
            _subscriptions: subscriptions,
        };
        // Re-read everything on each open so edits made by hand or via the CLI show up.
        this.config = config::load_config();
        this.db = store::load_db();
        this.windows = windows;
        this.reload_projects();
        this.refresh_git_status(cx);
        let mode = if this.config.editor.is_none() {
            Mode::Editors
        } else {
            Mode::Projects
        };
        this.set_mode(mode, cx);
        this.apply_theme(window, cx);
        this.status = cx
            .try_global::<ShortcutNotice>()
            .and_then(|notice| notice.0.clone())
            .map(|text| Status {
                text,
                problem: true,
                undo: false,
            });
        this
    }

    /// Closed without opening anything: keep the search for a while, in case
    /// that was by mistake (a click elsewhere).
    pub fn remember_search(&self, cx: &mut App) {
        if self.mode != Mode::Projects || self.closing {
            return;
        }
        let text = self.input.read(cx).text().to_string();
        cx.default_global::<LastSearch>().0 =
            (!text.trim().is_empty()).then(|| (text, Instant::now()));
    }

    /// Brings back the search remembered by `remember_search`, if it's recent,
    /// selected so that typing replaces it.
    pub fn restore_search(&mut self, cx: &mut Context<Self>) {
        let Some((text, at)) = cx.default_global::<LastSearch>().0.take() else {
            return;
        };
        if self.mode != Mode::Projects || at.elapsed() > KEEP_SEARCH {
            return;
        }
        self.set_query(&text, cx);
        self.input.update(cx, |input, cx| input.select_all_text(cx));
    }

    fn reload_projects(&mut self) {
        self.projects = store::collect(&self.config, &self.db);
        // Not in the database, so not collected.
        if let Some(group) = &self.unsaved
            && !self.projects.iter().any(|p| p.key() == group.key())
        {
            self.projects.push(group.clone());
        }
        let missing = self.projects.iter().any(|p| p.missing);
        self.commands = items::commands(update::is_installed(), missing);
        let detected = editors::detected_editors(false);
        self.names = self
            .db
            .editors
            .values()
            .flatten()
            .chain(self.config.editor.iter())
            .map(|command| (command.clone(), editors::editor_name(command, &detected)))
            .collect();
        // Found once here, as finding one looks on disk. The global editor's
        // too, for the icon at the start of the rows.
        let mut apps = HashMap::new();
        let global = self.config.editor.as_ref().filter(|e| !e.trim().is_empty());
        let own = self.projects.iter().filter_map(|p| p.editor.as_ref());
        for command in own.chain(global) {
            if !apps.contains_key(command) {
                let app = detected
                    .iter()
                    .find(|e| &e.command == command)
                    .map_or_else(|| editors::app_path(command), |e| e.app.clone());
                apps.insert(command.clone(), app);
            }
        }
        self.apps = apps;
        self.match_windows();
        self.open_first();
    }

    /// Puts the projects with an editor window open first: the one in view
    /// (its window was in front when the palette opened), then the others,
    /// the one used last first. The rest keep their order: the one opened
    /// last first.
    fn open_first(&mut self) {
        // Per project, its best window: in front first, then by recent use.
        let mut rank: HashMap<usize, (bool, usize)> = HashMap::new();
        for (w, project) in self.window_projects.iter().enumerate() {
            let Some(project) = *project else {
                continue;
            };
            let (front, z) = self.windows[w].rank();
            let this = (!front, z);
            rank.entry(project)
                .and_modify(|best| *best = (*best).min(this))
                .or_insert(this);
        }
        if rank.is_empty() {
            return;
        }
        let mut order: Vec<usize> = (0..self.projects.len()).collect();
        // Projects without a window last, in the order they were in.
        let last = (true, usize::MAX);
        order.sort_by_key(|&ix| (rank.get(&ix).copied().unwrap_or(last), ix));
        let mut slots: Vec<Option<Project>> = std::mem::take(&mut self.projects)
            .into_iter()
            .map(Some)
            .collect();
        self.projects = order
            .into_iter()
            .filter_map(|ix| slots[ix].take())
            .collect();
        // `window_projects` points into the old order.
        self.match_windows();
    }

    /// Whether a project has an editor window open, for the line between
    /// those and the rest while the list shows them all in order.
    fn is_open(&self, ix: usize) -> bool {
        self.open_keys.contains(&self.projects[ix].key())
    }

    /// Shows the last known `git status` of each repository straight away,
    /// and reads it again in the background where that's old.
    fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        let mut due: Vec<PathBuf> = Vec::new();
        for project in self.projects.iter().filter(|p| p.branch.is_some()) {
            let (status, stale) = git::cached_status(&project.path);
            if let Some(status) = status {
                self.git_status.insert(project.path.clone(), status);
            }
            if stale && !due.contains(&project.path) {
                due.push(project.path.clone());
            }
        }
        let reads: Vec<_> = due
            .into_iter()
            .map(|path| {
                cx.background_executor().spawn(async move {
                    let status = git::refresh_status(&path);
                    (path, status)
                })
            })
            .collect();
        if reads.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            for read in reads {
                let (path, status) = read.await;
                let Some(status) = status else {
                    continue;
                };
                let updated = this.update(cx, |this, cx| {
                    if this.git_status.insert(path, status) != Some(status) {
                        cx.notify();
                    }
                });
                // Closed: drop the rest.
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn name_of(&self, command: &str) -> String {
        self.names
            .get(command)
            .cloned()
            .unwrap_or_else(|| editors::editor_label(command))
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.show_shortcuts = false;
        self.actions_for = None;
        match mode {
            Mode::Projects => {
                self.open_with = None;
                self.browse = None;
                self.editing = None;
                self.new_from = None;
                self.forge_pick = None;
                self.group_page = None;
                self.expanded = None;
                // An unsaved group goes from the list again.
                if let Some(group) = self.unsaved.take() {
                    let key = group.key();
                    self.projects.retain(|p| p.key() != key);
                    self.match_windows();
                }
                self.arrange_rows();
            }
            Mode::Browse
            | Mode::Rename
            | Mode::Tags
            | Mode::AddCommand
            | Mode::NewProject
            | Mode::Switch
            | Mode::Forges
            | Mode::Group => {}
            Mode::Templates => {
                self.templates = self
                    .config
                    .templates
                    .iter()
                    .filter_map(|t| Template::parse(t))
                    .collect();
            }
            Mode::Editors => {
                self.editors = editors::detected_editors(true)
                    .into_iter()
                    .map(EditorOption::Detected)
                    .chain([EditorOption::Browse, EditorOption::FileManager])
                    .collect();
            }
            Mode::OpenWith => {
                // The project's own default first, then the global one, then the rest.
                let own = self.open_with_project().and_then(|p| p.editor.clone());
                let mut options: Vec<Editor> = own
                    .map(|command| Editor::new(self.name_of(&command), command))
                    .into_iter()
                    .collect();
                let global = self
                    .config
                    .editor
                    .clone()
                    .filter(|e| !e.trim().is_empty())
                    .map(|command| Editor::new(self.name_of(&command), command));
                for editor in global.into_iter().chain(editors::detected_editors(false)) {
                    if !options.iter().any(|o| o.command == editor.command) {
                        options.push(editor);
                    }
                }
                self.editors = options
                    .into_iter()
                    .map(EditorOption::Detected)
                    .chain([EditorOption::Browse])
                    .collect();
            }
        }
        let placeholder = match mode {
            // The rest (pasting a path or git URL…) is under F1.
            Mode::Projects => {
                format!(
                    "Search projects…   > commands · {SWITCH_PREFIX} windows · {FILES_PREFIX} files · # tags"
                )
            }
            Mode::Editors if self.config.editor.is_none() => {
                "Choose the editor to open projects with…".into()
            }
            Mode::Editors => "Default editor for all projects…".into(),
            Mode::OpenWith => "Open with…".into(),
            Mode::Browse => match &self.browse {
                Some(browse) => format!("Search in {}…", browse.breadcrumb()),
                None => String::new(),
            },
            Mode::Rename => "Leave empty to use the folder name".into(),
            Mode::Tags => "Tags, e.g. work oss; leave empty for none".into(),
            Mode::AddCommand => "A command to run in the project's folder, e.g. npm run dev".into(),
            Mode::Templates => "New project from…".into(),
            Mode::Group => "Search projects to tick… (#tag works too)".into(),
            Mode::Forges => match &self.forge_pick {
                Some(pick) => format!("What does {} run?", pick.host),
                None => String::new(),
            },
            Mode::NewProject => "Name of the new project's folder".into(),
            Mode::Switch => "Switch to…".into(),
        };
        self.input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        // Start from the current name or tags, selected so typing replaces them.
        let start = match mode {
            Mode::Rename => self.edited_project().map(|p| p.name.clone()),
            Mode::Tags => self.edited_project().map(|p| p.tags.join(" ")),
            _ => None,
        };
        match start {
            Some(text) => {
                self.set_query(&text, cx);
                self.input.update(cx, |input, cx| input.select_all_text(cx));
            }
            None => self.set_query("", cx),
        }
    }

    fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.query = query.to_string();
        self.input
            .update(cx, |input, cx| input.set_text(query.to_string(), cx));
        self.refilter(cx);
    }

    fn list(&self) -> List {
        match self.mode {
            Mode::Editors => List::Editors,
            Mode::OpenWith => List::OpenWith,
            Mode::Browse => List::Browse,
            Mode::Rename | Mode::Tags | Mode::AddCommand | Mode::NewProject => List::Text,
            Mode::Templates => List::Templates,
            Mode::Forges => List::Forges,
            Mode::Group => List::Group,
            Mode::Switch => List::Switch,
            Mode::Projects if self.query.starts_with('>') => List::Commands,
            // The switcher's list, searchable, without opening it by its shortcut.
            Mode::Projects if self.query.starts_with(SWITCH_PREFIX) => List::Switch,
            Mode::Projects if self.query.starts_with(FILES_PREFIX) => List::Files,
            Mode::Projects => List::Projects,
        }
    }

    fn open_with_project(&self) -> Option<&Project> {
        let key = self.open_with.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    /// The entry being renamed, tagged or given a command.
    fn edited_project(&self) -> Option<&Project> {
        let key = self.editing.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    /// What the list is filtered by: the query without a `>`, `@` or `$` in front.
    fn filter_query(&self) -> &str {
        match self.list() {
            List::Commands => self.query[1..].trim(),
            List::Files => self.query[FILES_PREFIX.len_utf8()..].trim(),
            List::Switch if self.mode == Mode::Projects => {
                self.query[SWITCH_PREFIX.len_utf8()..].trim()
            }
            _ => &self.query,
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let list = self.list();
        // Left the `@` list from inside a project's windows: all rows next time.
        if list != List::Switch && self.expanded.take().is_some() {
            self.arrange_rows();
        }
        if list == List::Files {
            self.pasted = None;
            self.clone_candidate = None;
            return self.search_files(cx);
        }
        // `matches` is about to be another list's.
        self.files.hits.clear();
        let query = self.filter_query().to_string();
        // Projects: `#tag` words keep the ones tagged with them, by the start of
        // the tag ("#wo" for "work"); the rest of the text is searched as usual.
        let (tags, text) = match list {
            List::Projects | List::Group => split_tags(&query),
            _ => (Vec::new(), query.clone()),
        };
        let candidates: Vec<usize> = (0..self.item_count())
            // Groups don't go in groups, and a folder that's gone can't open.
            .filter(|&ix| {
                list != List::Group
                    || !(self.projects[ix].is_workspace() || self.projects[ix].missing)
            })
            .filter(|&ix| tags.is_empty() || self.projects[ix].has_tags(&tags))
            .collect();
        self.matches.clear();
        if text.is_empty() {
            self.matches.extend(candidates.into_iter().map(|ix| Match {
                ix,
                title_hl: Vec::new(),
                subtitle_hl: Vec::new(),
            }));
        } else {
            let query = text;
            let mut scored: Vec<(fuzzy::Rank, Match)> = candidates
                .into_iter()
                .filter_map(|ix| {
                    let (title, subtitle) = self.item_text(ix);
                    // Projects: the ones used more, and more lately, first
                    // among matches as good.
                    let boost = match list {
                        List::Projects => self.projects[ix].search_boost(),
                        _ => 0.,
                    };
                    let (rank, m) = fuzzy::rank_item(&query, &title, &subtitle, boost)?;
                    Some((
                        rank,
                        Match {
                            ix,
                            title_hl: m.title_hl,
                            subtitle_hl: m.subtitle_hl,
                        },
                    ))
                })
                .collect();
            scored.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
            self.matches.extend(scored.into_iter().map(|(_, m)| m));
        }

        self.pasted = (list == List::Projects && looks_like_path(&query))
            .then(|| paths::normalize(&query))
            .flatten()
            .and_then(|path| {
                if path.is_file() {
                    Some(PastedPath::File(path))
                } else if path.is_dir()
                    && !self
                        .projects
                        .iter()
                        .any(|p| !p.is_workspace() && p.path == path)
                {
                    Some(PastedPath::Folder(path))
                } else {
                    None
                }
            });
        self.clone_candidate = (list == List::Projects)
            .then(|| git::clone_name(&query))
            .flatten()
            .map(|name| CloneTarget {
                url: query.clone(),
                name,
                into: self.config.scan_dirs.first().cloned(),
            });

        self.selected = 0;
        self.reset_list();
        cx.notify();
    }

    fn select(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.show_shortcuts {
            return self.select_shortcut(delta, cx);
        }
        if self.menu_open() {
            return self.select_in_menu(delta, cx);
        }
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        let row = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        self.select_row(row, cx);
    }

    /// Page down (`delta` 1) or up (-1): a list's height of rows at a time,
    /// stopping at the ends rather than going round.
    fn select_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.menu_open() {
            return self.select_in_menu(delta * 6, cx);
        }
        // One less than the list shows, so the row at the edge stays in sight.
        let height = f32::from(self.list_state.viewport_bounds().size.height);
        let page = ((height / (ROW_HEIGHT + ROW_GAP)) as usize)
            .saturating_sub(1)
            .max(1);
        let row = if delta < 0 {
            self.selected.saturating_sub(page)
        } else {
            self.selected + page
        };
        self.select_row(row, cx);
    }

    /// Highlights `row` (or the last one) and scrolls to it.
    fn select_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        self.selected = row.min(len - 1);
        self.scroll_to_row(self.selected);
        cx.notify();
    }

    /// Moves the selection to the first row matching `f`, if any.
    fn select_where(&mut self, f: impl Fn(&Self, usize) -> bool) {
        if let Some(row) = self.matches.iter().position(|m| f(self, m.ix)) {
            self.selected = row;
            self.scroll_to_row(row);
        }
    }

    fn selected_project(&self) -> Option<&Project> {
        if self.list() != List::Projects {
            return None;
        }
        self.matches
            .get(self.selected)
            .map(|m| &self.projects[m.ix])
    }

    fn selected_editor(&self) -> Option<&Editor> {
        if self.list() != List::OpenWith {
            return None;
        }
        match self.matches.get(self.selected).map(|m| &self.editors[m.ix]) {
            Some(EditorOption::Detected(editor)) => Some(editor),
            _ => None,
        }
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_shortcuts {
            return self.run_shortcut(window, cx);
        }
        if self.menu_open() {
            return self.confirm_menu(window, cx);
        }
        // A pasted path or git URL takes enter over the matches below it.
        if self.pasted.is_some() {
            return self.open_pasted(true, window, cx);
        }
        if let Some(target) = self.clone_candidate.clone() {
            return self.clone_repo(target, window, cx);
        }
        if self.list() == List::Text {
            return match self.mode {
                Mode::Tags => self.finish_tags(cx),
                Mode::AddCommand => self.finish_add_command(cx),
                Mode::NewProject => self.finish_new_project(window, cx),
                _ => self.finish_rename(cx),
            };
        }
        if self.list() == List::Group {
            return self.confirm_group(window, cx);
        }
        let Some(ix) = self.matches.get(self.selected).map(|m| m.ix) else {
            return;
        };
        match self.list() {
            List::Editors => self.choose_editor(ix, window, cx),
            List::OpenWith => self.choose_open_with(ix, window, cx),
            List::Commands => self.run_command(self.commands[ix], window, cx),
            // A project's row: its window used last.
            List::Switch => self.switch_to(self.switch_rows[ix][0], window, cx),
            List::Templates => {
                self.new_from = Some(self.templates[ix].clone());
                self.set_mode(Mode::NewProject, cx);
            }
            List::Forges => self.choose_forge(ix, window, cx),
            List::Browse | List::Files => {
                if let Some((project, ..)) = self.selected_file() {
                    let editor = project.default_editor(&self.config);
                    self.launch_entry(Target::Editor(editor), window, cx);
                }
            }
            List::Projects => {
                let entry = if self.marked.len() > 1 {
                    self.marked_group()
                } else {
                    self.selected_project().cloned()
                };
                if let Some(entry) = entry {
                    self.open_entry(entry, window, cx);
                }
            }
            List::Text | List::Group => {}
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_shortcuts {
            self.show_shortcuts = false;
            cx.notify();
            return;
        }
        if self.menu_open() {
            return self.close_menu(cx);
        }
        match self.list() {
            List::Commands | List::Files => self.set_query("", cx),
            // A project's windows: back to every project's.
            List::Switch if self.expanded.is_some() => {
                self.leave(cx);
            }
            List::Switch if self.mode == Mode::Projects => self.set_query("", cx),
            List::Browse => self.exit_browse(cx),
            List::OpenWith | List::Text | List::Templates | List::Forges | List::Group => {
                self.back_to_projects(cx);
            }
            List::Projects if !self.marked.is_empty() => {
                self.marked.clear();
                cx.notify();
            }
            List::Editors if self.config.editor.is_some() => self.back_to_projects(cx),
            _ => {
                self.remember_search(cx);
                window.remove_window();
            }
        }
    }

    /// The page the list is on, for the search bar, after its back button;
    /// `None` on the project list and the lists that don't go back to it.
    fn page_title(&self) -> Option<String> {
        if self.list() == List::Switch {
            let project = &self.projects[self.expanded?];
            return Some(format!("{}'s windows", project.name));
        }
        Some(match self.mode {
            Mode::Browse => self.browse.as_ref()?.breadcrumb(),
            Mode::OpenWith => format!("Open {} with", self.open_with_project()?.name),
            Mode::Rename => format!("Rename {}", self.edited_project()?.name),
            Mode::Tags => format!("Tags for {}", self.edited_project()?.name),
            Mode::AddCommand => format!("Command for {}", self.edited_project()?.name),
            Mode::Templates => "New project".into(),
            Mode::Forges => format!("What runs {}", self.forge_pick.as_ref()?.host),
            Mode::Group => match &self.group_page.as_ref()?.editing {
                Some(folders) => {
                    let key = store::entry_key(folders);
                    let group = self.projects.iter().find(|p| p.key() == key)?;
                    format!("Folders of {}", group.name)
                }
                None => "Open together".into(),
            },
            Mode::NewProject => format!("New from {}", self.new_from.as_ref()?.name()),
            Mode::Editors if self.config.editor.is_some() => "Default editor".into(),
            Mode::Projects | Mode::Editors | Mode::Switch => return None,
        })
    }

    /// The back button, or backspace in an empty search box: up a folder, out
    /// of a project's windows, or back to the projects. Returns whether there
    /// was somewhere to go back to.
    fn go_back(&mut self, cx: &mut Context<Self>) -> bool {
        if self.page_title().is_none() {
            return false;
        }
        match self.list() {
            List::Browse | List::Switch => self.leave(cx),
            _ => {
                self.back_to_projects(cx);
                true
            }
        }
    }

    /// Back to the project list, with the entry that was being edited selected.
    fn back_to_projects(&mut self, cx: &mut Context<Self>) {
        let key = self
            .open_with
            .clone()
            .or_else(|| self.editing.clone())
            .or_else(|| self.actions_for.clone())
            .or_else(|| self.forge_pick.as_ref().map(|p| p.key.clone()))
            .or_else(|| self.group_page.as_ref().map(|p| p.from.clone()));
        self.set_mode(Mode::Projects, cx);
        self.select_where(|this, ix| Some(this.projects[ix].key()) == key);
    }

    /// Takes the latest update state, which the update command shows.
    fn sync_update(&mut self, cx: &mut Context<Self>) {
        let Some(updates) = cx.try_global::<Updates>() else {
            return;
        };
        self.update = updates.state.clone();
        self.refresh_commands(cx);
    }

    /// Picks the colours for `config.theme` and the system setting.
    fn apply_theme(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.theme = Theme::for_setting(self.config.theme, window.appearance());
        let theme = self.theme;
        self.input.update(cx, |input, cx| {
            input.set_colors(theme.placeholder, theme.accent, cx);
        });
        self.refresh_commands(cx);
    }

    /// A command's title changed (update state, theme): filter the `>` list
    /// again, as match highlights point into the old title, keeping the same
    /// command selected.
    fn refresh_commands(&mut self, cx: &mut Context<Self>) {
        if self.list() == List::Commands {
            let selected = self.matches.get(self.selected).map(|m| self.commands[m.ix]);
            self.refilter(cx);
            self.select_where(|this, ix| Some(this.commands[ix]) == selected);
        }
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        // A change after a remove: undoing it now would lose this one.
        self.undo = None;
        if let Err(err) = store::save_db(&self.db) {
            self.problem(format!("Could not save: {err}"), cx);
        }
    }

    /// Ctrl went down (`down`) or up. The numbers show once it has been held
    /// for a moment, so ctrl shortcuts like ctrl-k don't flash them.
    fn ctrl_held(&mut self, down: bool, cx: &mut Context<Self>) {
        // The numbers are for the project list, not the menu over it.
        let down = down && !self.menu_open();
        self.ctrl_down = down;
        if !down {
            if self.numbers_shown {
                self.numbers_shown = false;
                cx.notify();
            }
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            this.update(cx, |this, cx| {
                if this.ctrl_down && !this.numbers_shown {
                    this.numbers_shown = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// A passing message in the footer, e.g. "Renamed to proj", gone after a few
    /// seconds (unless another message took its place).
    fn notice(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_status(text, false, false, Some(Duration::from_secs(4)), cx);
    }

    /// What can't be done or went wrong, until the next key.
    fn problem(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_status(text, true, false, None, cx);
    }

    /// A message that stays until the next one, e.g. "Cloning into…".
    fn say(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.show_status(text, false, false, None, cx);
    }

    /// `shown`: for how long, or `None` until something replaces it.
    fn show_status(
        &mut self,
        text: impl Into<SharedString>,
        problem: bool,
        undo: bool,
        shown: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        let status = Status {
            text: text.into(),
            problem,
            undo,
        };
        self.status = Some(status.clone());
        cx.notify();
        let Some(shown) = shown else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown).await;
            this.update(cx, |this, cx| {
                if this.status.as_ref() == Some(&status) {
                    this.status = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// Typed first in the project search, lists the open editor windows instead
/// (`>` lists the commands).
const SWITCH_PREFIX: char = '@';

fn secondary() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}

/// The `#tag` words of a search (lowercase, without the `#`), and the rest of it.
fn split_tags(query: &str) -> (Vec<String>, String) {
    let (tags, words): (Vec<&str>, Vec<&str>) = query
        .split_whitespace()
        .partition(|word| word.len() > 1 && word.starts_with('#'));
    (
        tags.iter().map(|t| t[1..].to_lowercase()).collect(),
        words.join(" "),
    )
}

/// Paths start with `~`, `/`, `\` or a drive letter (`C:`).
fn looks_like_path(query: &str) -> bool {
    let bytes = query.as_bytes();
    matches!(bytes.first(), Some(b'~' | b'/' | b'\\'))
        || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_in_the_search() {
        let split = |query: &str| split_tags(query);
        assert_eq!(
            split("#Work api"),
            (vec!["work".to_string()], "api".to_string())
        );
        assert_eq!(
            split("client #oss #web"),
            (
                vec!["oss".to_string(), "web".to_string()],
                "client".to_string()
            )
        );
        // A lone `#` is searched for like any other character.
        assert_eq!(split("# api"), (Vec::new(), "# api".to_string()));
        assert_eq!(split("app"), (Vec::new(), "app".to_string()));
    }
}
