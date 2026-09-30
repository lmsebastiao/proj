//! The search dialog: state, modes, filtering and selection. Actions live in
//! `projects`, `editor_choice` and `browse`, drawing in `render`, and the list
//! of keys in `shortcuts`.

mod browse;
mod editor_choice;
mod items;
mod keymap;
mod projects;
mod render;
mod shortcuts;
mod theme;

use std::{collections::HashMap, path::PathBuf};

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Global, ScrollStrategy, SharedString,
    Subscription, UniformListScrollHandle, Window, prelude::*,
};

use crate::{
    autostart,
    config::{self, Config},
    editors::{self, Editor},
    fuzzy, git,
    input::{self, TextInput},
    paths,
    store::{self, Db, Project},
};

use items::{COMMANDS, CloneTarget, EditorOption, List, Match, Mode, Target};
use keymap::{Confirm, Dismiss};

pub use keymap::bind_keys;

/// Shortcuts that couldn't be registered, shown in the footer each time the
/// palette opens until `hotkey` in config.toml is fixed.
pub struct ShortcutNotice(pub Option<SharedString>);

impl Global for ShortcutNotice {}

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
    /// Key of the entry being opened in `Mode::OpenWith`, or the one that was
    /// selected when `Mode::Editors` opened.
    open_with: Option<String>,
    /// The project being browsed in `Mode::Browse`.
    browse: Option<browse::Browse>,
    /// Key of the entry being renamed in `Mode::Rename`.
    renaming: Option<String>,
    /// Projects marked with tab, to open together as one workspace.
    marked: Vec<PathBuf>,
    autostart: bool,
    matches: Vec<Match>,
    selected: usize,
    /// Set when the query is a path to an existing, not-yet-listed folder.
    add_candidate: Option<PathBuf>,
    /// Set when the query is a git URL.
    clone_candidate: Option<CloneTarget>,
    /// A native file dialog is open; don't treat the lost focus as a dismissal.
    picking: bool,
    /// `git clone` is running. The palette stays open so it can open the result.
    cloning: bool,
    /// The dropdown with every shortcut (F1) is open.
    show_shortcuts: bool,
    status: Option<SharedString>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("", cx));
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
            // Behave like a launcher: clicking anywhere else dismisses it.
            cx.observe_window_activation(window, |this, window, _| {
                if !this.picking && !this.cloning && !window.is_window_active() {
                    window.remove_window();
                }
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
            open_with: None,
            browse: None,
            renaming: None,
            marked: Vec::new(),
            autostart: autostart::is_enabled(),
            matches: Vec::new(),
            selected: 0,
            add_candidate: None,
            clone_candidate: None,
            picking: false,
            cloning: false,
            show_shortcuts: false,
            status: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        // Re-read everything on each open so edits made by hand or via the CLI show up.
        this.config = config::load_config();
        this.db = store::load_db();
        this.reload_projects();
        let mode = if this.config.editor.is_none() {
            Mode::Editors
        } else {
            Mode::Projects
        };
        this.set_mode(mode, cx);
        this.status = cx
            .try_global::<ShortcutNotice>()
            .and_then(|notice| notice.0.clone());
        this
    }

    fn reload_projects(&mut self) {
        self.projects = store::collect(&self.config, &self.db);
        let detected = editors::detected_editors(false);
        self.names = self
            .db
            .editors
            .values()
            .flatten()
            .chain(self.config.editor.iter())
            .map(|command| (command.clone(), editors::editor_name(command, &detected)))
            .collect();
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
        match mode {
            Mode::Projects => {
                self.open_with = None;
                self.browse = None;
                self.renaming = None;
            }
            Mode::Browse | Mode::Rename => {}
            Mode::Editors => {
                self.editors = editors::detected_editors(true)
                    .into_iter()
                    .map(EditorOption::Detected)
                    .chain([EditorOption::Browse, EditorOption::FileManager])
                    .collect();
            }
            Mode::OpenWith => {
                // The project's own editors first (in order), then everything else.
                let own = self
                    .open_with_project()
                    .map(|p| p.editors.clone())
                    .unwrap_or_default();
                let mut options: Vec<Editor> = own
                    .iter()
                    .map(|command| Editor {
                        name: self.name_of(command),
                        command: command.clone(),
                    })
                    .collect();
                let global = self
                    .config
                    .editor
                    .clone()
                    .filter(|e| !e.trim().is_empty())
                    .map(|command| Editor {
                        name: self.name_of(&command),
                        command,
                    });
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
            Mode::Projects => {
                "Search projects, > for commands, or paste a folder path or git URL…".into()
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
        };
        self.input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        if mode == Mode::Rename {
            // Start from the current name, selected so typing replaces it.
            let name = self
                .renamed_project()
                .map(|p| p.name.clone())
                .unwrap_or_default();
            self.set_query(&name, cx);
            self.input.update(cx, |input, cx| input.select_all_text(cx));
        } else {
            self.set_query("", cx);
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
            Mode::Rename => List::Rename,
            Mode::Projects if self.query.starts_with('>') => List::Commands,
            Mode::Projects => List::Projects,
        }
    }

    fn open_with_project(&self) -> Option<&Project> {
        let key = self.open_with.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    fn renamed_project(&self) -> Option<&Project> {
        let key = self.renaming.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let list = self.list();
        let query = match list {
            List::Commands => self.query[1..].trim().to_string(),
            _ => self.query.clone(),
        };
        self.matches.clear();
        if query.is_empty() {
            self.matches.extend((0..self.item_count()).map(|ix| Match {
                ix,
                title_hl: Vec::new(),
                subtitle_hl: Vec::new(),
            }));
        } else {
            let mut scored: Vec<(i32, Match)> = (0..self.item_count())
                .filter_map(|ix| {
                    let (title, subtitle) = self.item_text(ix);
                    let boost = match list {
                        List::Projects => self.projects[ix].search_boost(),
                        _ => 0,
                    };
                    let m = fuzzy::score_item(&query, &title, &subtitle, boost)?;
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
            self.matches.extend(scored.into_iter().map(|(_, m)| m));
        }

        self.add_candidate = (list == List::Projects && looks_like_path(&query))
            .then(|| paths::normalize(&query))
            .flatten()
            .filter(|path| {
                path.is_dir()
                    && !self
                        .projects
                        .iter()
                        .any(|p| !p.is_workspace() && &p.path == path)
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
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn select(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.matches.len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    /// Moves the selection to the first row matching `f`, if any.
    fn select_where(&mut self, f: impl Fn(&Self, usize) -> bool) {
        if let Some(row) = self.matches.iter().position(|m| f(self, m.ix)) {
            self.selected = row;
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
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
        // A pasted folder or git URL takes enter over the matches below it.
        if let Some(path) = self.add_candidate.take() {
            return self.add_paths(vec![path], cx);
        }
        if let Some(target) = self.clone_candidate.clone() {
            return self.clone_repo(target, window, cx);
        }
        if self.list() == List::Rename {
            return self.finish_rename(cx);
        }
        let Some(ix) = self.matches.get(self.selected).map(|m| m.ix) else {
            return;
        };
        match self.list() {
            List::Editors => self.choose_editor(ix, window, cx),
            List::OpenWith => self.choose_open_with(ix, window, cx),
            List::Commands => self.run_command(COMMANDS[ix], window, cx),
            List::Browse => {
                if let Some(browse) = &self.browse {
                    let editor = browse.project.default_editor(&self.config);
                    self.launch_entry(Target::Editor(editor), window, cx);
                }
            }
            List::Projects => {
                let entry = if self.marked.len() > 1 {
                    self.marked_workspace(cx)
                } else {
                    self.selected_project().cloned()
                };
                if let Some(entry) = entry {
                    self.open_entry(entry, window, cx);
                }
            }
            List::Rename => {}
        }
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.show_shortcuts {
            self.show_shortcuts = false;
            cx.notify();
            return;
        }
        match self.list() {
            List::Commands => self.set_query("", cx),
            List::Browse => self.exit_browse(cx),
            List::OpenWith | List::Rename => self.back_to_projects(cx),
            List::Projects if !self.marked.is_empty() => {
                self.marked.clear();
                cx.notify();
            }
            List::Editors if self.config.editor.is_some() => self.back_to_projects(cx),
            _ => window.remove_window(),
        }
    }

    /// Back to the project list, with the entry that was being edited selected.
    fn back_to_projects(&mut self, cx: &mut Context<Self>) {
        let key = self.open_with.clone().or_else(|| self.renaming.clone());
        self.set_mode(Mode::Projects, cx);
        self.select_where(|this, ix| Some(this.projects[ix].key()) == key);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = store::save_db(&self.db) {
            self.status = Some(format!("Could not save: {err}").into());
            cx.notify();
        }
    }
}

fn secondary() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
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
