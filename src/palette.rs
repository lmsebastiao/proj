//! The search dialog.

use std::{collections::HashMap, ops::Range, path::PathBuf};

use gpui::{
    AnyElement, App, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight,
    HighlightStyle, KeyBinding, PathPromptOptions, ScrollStrategy, SharedString, StyledText,
    Subscription, UniformListScrollHandle, Window, actions, div, prelude::*, px, rgb, uniform_list,
};

use crate::{
    autostart, fuzzy,
    input::{self, TextInput},
    open::{self, Editor},
    store::{self, Config, Db, Project},
};

actions!(
    palette,
    [
        SelectNext,
        SelectPrev,
        Confirm,
        Reveal,
        OpenTerminal,
        OpenWithMenu,
        TogglePin,
        CopyPath,
        Remove,
        AddProjects,
        ChooseEditor,
        Dismiss,
        QuitApp
    ]
);

pub fn bind_keys(cx: &mut App) {
    input::bind_keys(cx);
    let ctx = Some("Palette");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("ctrl-n", SelectNext, ctx),
        KeyBinding::new("tab", SelectNext, ctx),
        KeyBinding::new("up", SelectPrev, ctx),
        KeyBinding::new("ctrl-p", SelectPrev, ctx),
        KeyBinding::new("shift-tab", SelectPrev, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("secondary-enter", Reveal, ctx),
        KeyBinding::new("shift-enter", OpenTerminal, ctx),
        KeyBinding::new("alt-enter", OpenWithMenu, ctx),
        KeyBinding::new("secondary-s", TogglePin, ctx),
        KeyBinding::new("secondary-shift-c", CopyPath, ctx),
        KeyBinding::new("secondary-d", Remove, ctx),
        KeyBinding::new("shift-delete", Remove, ctx),
        KeyBinding::new("secondary-o", AddProjects, ctx),
        KeyBinding::new("secondary-e", ChooseEditor, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        KeyBinding::new("secondary-q", QuitApp, ctx),
    ]);
    cx.on_action(|_: &QuitApp, cx| cx.quit());
}

const BG: u32 = 0x1f2023;
const BORDER: u32 = 0x34363c;
const TEXT: u32 = 0xdcdfe4;
const MUTED: u32 = 0x8b8f98;
const BRANCH: u32 = 0xb4a0e0;
const SELECTED: u32 = 0x2c3038;
const ACCENT: u32 = 0x74ade8;
const ROW_HEIGHT: f32 = 46.;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Projects,
    /// Choosing the global editor.
    Editors,
    /// Choosing an editor for one project (`Palette::open_with`).
    OpenWith,
}

/// What the list currently shows. Commands appear when the query starts with `>`.
#[derive(Clone, Copy, PartialEq)]
enum List {
    Projects,
    Editors,
    OpenWith,
    Commands,
}

enum EditorOption {
    Detected(Editor),
    Browse,
    FileManager,
}

#[derive(Clone, Copy)]
enum PaletteCommand {
    Autostart,
    AddProjects,
    ChangeEditor,
    OpenConfig,
    Quit,
}

const COMMANDS: [PaletteCommand; 5] = [
    PaletteCommand::Autostart,
    PaletteCommand::AddProjects,
    PaletteCommand::ChangeEditor,
    PaletteCommand::OpenConfig,
    PaletteCommand::Quit,
];

enum Target {
    Editor(String),
    FileManager,
    Terminal,
}

struct Match {
    ix: usize,
    title_hl: Vec<usize>,
    subtitle_hl: Vec<usize>,
}

/// Text shown on the right of a row.
struct Meta {
    top: Option<(String, u32)>,
    bottom: Option<String>,
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
    /// The project being opened in `Mode::OpenWith`.
    open_with: Option<PathBuf>,
    autostart: bool,
    matches: Vec<Match>,
    selected: usize,
    /// Set when the query is a path to an existing, not-yet-listed folder.
    add_candidate: Option<PathBuf>,
    /// A native file dialog is open; don't treat the lost focus as a dismissal.
    picking: bool,
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
                    this.refilter(cx);
                }
            }),
            // Behave like a launcher: clicking anywhere else dismisses it.
            cx.observe_window_activation(window, |this, window, _| {
                if !this.picking && !window.is_window_active() {
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
            autostart: autostart::is_enabled(),
            matches: Vec::new(),
            selected: 0,
            add_candidate: None,
            picking: false,
            status: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        // Re-read everything on each open so edits made by hand or via the CLI show up.
        this.config = store::load_config();
        this.db = store::load_db();
        this.reload_projects();
        let mode = if this.config.editor.is_none() {
            Mode::Editors
        } else {
            Mode::Projects
        };
        this.set_mode(mode, cx);
        this
    }

    fn reload_projects(&mut self) {
        self.projects = store::collect(&self.config, &self.db);
        let detected = open::detected_editors(false);
        self.names = self
            .db
            .editors
            .values()
            .flatten()
            .chain(self.config.editor.iter())
            .map(|command| (command.clone(), open::editor_name(command, &detected)))
            .collect();
    }

    fn name_of(&self, command: &str) -> String {
        self.names
            .get(command)
            .cloned()
            .unwrap_or_else(|| open::editor_label(command))
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        match mode {
            Mode::Projects => self.open_with = None,
            Mode::Editors => {
                self.editors = open::detected_editors(true)
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
                for editor in global.into_iter().chain(open::detected_editors(false)) {
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
            Mode::Projects => "Search projects, > for commands, or paste a folder path…",
            Mode::Editors => "Choose the editor to open projects with…",
            Mode::OpenWith => "Open with…",
        };
        self.input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        self.set_query("", cx);
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
            Mode::Projects if self.query.starts_with('>') => List::Commands,
            Mode::Projects => List::Projects,
        }
    }

    fn open_with_project(&self) -> Option<&Project> {
        let path = self.open_with.as_ref()?;
        self.projects.iter().find(|p| &p.path == path)
    }

    /// Title and subtitle of an item in the current list.
    fn item_text(&self, ix: usize) -> (String, String) {
        match self.list() {
            List::Projects => {
                let project = &self.projects[ix];
                (project.name.clone(), store::display_path(&project.path))
            }
            List::Editors | List::OpenWith => match &self.editors[ix] {
                EditorOption::Detected(editor) => {
                    let current = self.mode == Mode::Editors
                        && self.config.editor.as_deref() == Some(editor.command.as_str());
                    let title = if current {
                        format!("{}  (current)", editor.name)
                    } else {
                        editor.name.clone()
                    };
                    (title, store::display_path(editor.command.as_ref()))
                }
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
                        "Change default editor".into(),
                        format!(
                            "Currently {} · {m}-e",
                            self.name_of(self.config.editor.as_deref().unwrap_or(""))
                        ),
                    ),
                    PaletteCommand::OpenConfig => (
                        "Open config file".into(),
                        store::display_path(&store::config_path()),
                    ),
                    PaletteCommand::Quit => (
                        "Quit proj".into(),
                        format!("Stop the background launcher · {m}-q"),
                    ),
                }
            }
        }
    }

    /// Right-hand details: branch, editors and last opened for projects; the
    /// editor's role in the Open-with list.
    fn item_meta(&self, ix: usize, now: u64) -> Meta {
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
                    Some("project default")
                } else if own.contains(&editor.command) {
                    Some("this project")
                } else if is_global && own.is_empty() {
                    Some("default")
                } else if is_global {
                    Some("global default")
                } else {
                    None
                };
                Meta {
                    top: role.map(|r| (r.to_string(), ACCENT)),
                    bottom: None,
                }
            }
            List::Editors | List::Commands => Meta {
                top: None,
                bottom: None,
            },
        }
    }

    fn item_count(&self) -> usize {
        match self.list() {
            List::Projects => self.projects.len(),
            List::Editors | List::OpenWith => self.editors.len(),
            List::Commands => COMMANDS.len(),
        }
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
                    if let Some((score, hl)) = fuzzy::score(&query, &title) {
                        // Small boost so pinned and recently used projects win ties.
                        let boost = match list {
                            List::Projects => {
                                let p = &self.projects[ix];
                                (p.pinned as i32 + (p.last_opened > 0) as i32) * 8
                            }
                            _ => 0,
                        };
                        return Some((
                            score + 1000 + boost,
                            Match {
                                ix,
                                title_hl: hl,
                                subtitle_hl: Vec::new(),
                            },
                        ));
                    }
                    fuzzy::score(&query, &subtitle).map(|(score, hl)| {
                        (
                            score,
                            Match {
                                ix,
                                title_hl: Vec::new(),
                                subtitle_hl: hl,
                            },
                        )
                    })
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.matches.extend(scored.into_iter().map(|(_, m)| m));
        }

        self.add_candidate = (list == List::Projects && looks_like_path(&query))
            .then(|| store::normalize(&query))
            .flatten()
            .filter(|path| path.is_dir() && !self.projects.iter().any(|p| &p.path == path));

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
        let Some(ix) = self.matches.get(self.selected).map(|m| m.ix) else {
            if let Some(path) = self.add_candidate.take() {
                self.add_paths(vec![path], cx);
            }
            return;
        };
        match self.list() {
            List::Editors => self.choose_editor(ix, window, cx),
            List::OpenWith => self.choose_open_with(ix, window, cx),
            List::Commands => self.run_command(COMMANDS[ix], window, cx),
            List::Projects => match self.add_candidate.take() {
                Some(path) => self.add_paths(vec![path], cx),
                None => self.open_default(window, cx),
            },
        }
    }

    /// Enter on a project: its only editor, the global one, or ask when it has several.
    fn open_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        match project.editors.as_slice() {
            [] => {
                let editor = self.config.editor.clone().unwrap_or_default();
                self.launch(&project, Target::Editor(editor), window, cx);
            }
            [only] => self.launch(&project, Target::Editor(only.clone()), window, cx),
            _ => self.show_open_with(project.path, cx),
        }
    }

    fn show_open_with(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open_with = Some(path);
        self.set_mode(Mode::OpenWith, cx);
    }

    fn open_selected(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(project) = self.selected_project().cloned() {
            self.launch(&project, target, window, cx);
        }
    }

    fn launch(
        &mut self,
        project: &Project,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = match &target {
            Target::Editor(editor) => open::open_with(&self.config, editor, &project.path),
            Target::FileManager => open::reveal(&project.path),
            Target::Terminal => open::open_terminal(&project.path),
        };
        match result {
            Ok(()) => {
                self.db
                    .opened
                    .insert(project.path.to_string_lossy().into_owned(), store::now());
                self.save(cx);
                window.remove_window();
            }
            Err(err) => {
                let program = match &target {
                    Target::Editor(editor) => self.name_of(editor),
                    Target::FileManager => "the file manager".into(),
                    Target::Terminal => "a terminal".into(),
                };
                self.status = Some(format!("Failed to launch {program}: {err}").into());
                cx.notify();
            }
        }
    }

    fn choose_open_with(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.open_with_project().cloned() else {
            return;
        };
        match &self.editors[ix] {
            EditorOption::Detected(editor) => {
                let command = editor.command.clone();
                self.launch(&project, Target::Editor(command), window, cx);
            }
            EditorOption::Browse => {
                let options = PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Open with".into()),
                };
                // A program picked for this project joins its list.
                self.pick(options, window, cx, move |this, paths, window, cx| {
                    if let Some(path) = paths.into_iter().next() {
                        let command = path.to_string_lossy().into_owned();
                        this.edit_project_editors(&project.path, |list| list.push(command.clone()));
                        this.launch(&project, Target::Editor(command), window, cx);
                    }
                });
            }
            EditorOption::FileManager => {}
        }
    }

    /// Ctrl-S in Open-with: add/remove the selected editor for this project.
    fn toggle_project_editor(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(editor)) = (
            self.open_with_project().cloned(),
            self.selected_editor().cloned(),
        ) else {
            return;
        };
        let global = self.config.editor.clone();
        let removing = project.editors.contains(&editor.command);
        self.edit_project_editors(&project.path, |list| {
            if removing {
                list.retain(|c| c != &editor.command);
            } else {
                store::offer_editor(list, &editor.command, global.as_deref());
            }
        });
        let status = if removing {
            format!("Removed {} from {}", editor.name, project.name)
        } else {
            format!(
                "{} now offers {}",
                project.name,
                self.editor_list(&project.path)
            )
        };
        self.refresh_open_with(&editor.command, status, cx);
    }

    /// Ctrl-Enter in Open-with: make the selected editor this project's default.
    fn make_project_default(&mut self, cx: &mut Context<Self>) {
        let (Some(project), Some(editor)) = (
            self.open_with_project().cloned(),
            self.selected_editor().cloned(),
        ) else {
            return;
        };
        self.edit_project_editors(&project.path, |list| {
            store::make_default_editor(list, &editor.command)
        });
        let status = format!("{} opens {} by default", editor.name, project.name);
        self.refresh_open_with(&editor.command, status, cx);
    }

    fn edit_project_editors(
        &mut self,
        path: &std::path::Path,
        edit: impl FnOnce(&mut Vec<String>),
    ) {
        let key = path.to_string_lossy().into_owned();
        let list = self.db.editors.entry(key.clone()).or_default();
        edit(list);
        if list.is_empty() {
            self.db.editors.remove(&key);
        }
        if let Err(err) = store::save_db(&self.db) {
            self.status = Some(format!("Could not save: {err}").into());
        }
        self.reload_projects();
    }

    fn editor_list(&self, path: &std::path::Path) -> String {
        self.projects
            .iter()
            .find(|p| p.path == path)
            .map(|p| {
                p.editors
                    .iter()
                    .map(|c| self.name_of(c))
                    .collect::<Vec<_>>()
                    .join(" / ")
            })
            .unwrap_or_default()
    }

    /// Rebuilds the Open-with list after an edit, keeping `command` selected.
    fn refresh_open_with(&mut self, command: &str, status: String, cx: &mut Context<Self>) {
        self.set_mode(Mode::OpenWith, cx);
        self.select_where(|this, ix| {
            matches!(&this.editors[ix], EditorOption::Detected(e) if e.command == command)
        });
        self.status = Some(status.into());
        cx.notify();
    }

    fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        let pinned = !project.pinned;
        if pinned {
            self.db.pinned.insert(project.path.clone());
        } else {
            self.db.pinned.remove(&project.path);
        }
        self.save(cx);
        self.reload_projects();
        self.refilter(cx);
        // Keep the selection on the same project after re-sorting.
        self.select_where(|this, ix| this.projects[ix].path == project.path);
        let verb = if pinned { "Pinned" } else { "Unpinned" };
        self.status = Some(format!("{verb} {}", project.name).into());
    }

    fn copy_path(&mut self, _: &CopyPath, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(project) = self.selected_project() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                project.path.to_string_lossy().into_owned(),
            ));
            window.remove_window();
        }
    }

    fn remove(&mut self, _: &Remove, _: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        if project.manual {
            self.db.manual.retain(|p| p != &project.path);
        } else {
            self.db.hidden.insert(project.path.clone());
        }
        let key = project.path.to_string_lossy().into_owned();
        self.db.pinned.remove(&project.path);
        self.db.editors.remove(&key);
        self.db.opened.remove(&key);
        self.save(cx);
        self.projects.retain(|p| p.path != project.path);
        let selected = self.selected;
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.status = Some(format!("Removed {}", project.name).into());
    }

    fn run_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::Autostart => match autostart::set(!self.autostart) {
                Ok(()) => {
                    self.autostart = !self.autostart;
                    let state = if self.autostart { "on" } else { "off" };
                    self.status = Some(format!("Start on login turned {state}").into());
                    cx.notify();
                }
                Err(err) => {
                    self.status = Some(format!("Could not change start on login: {err}").into());
                    cx.notify();
                }
            },
            PaletteCommand::AddProjects => {
                self.set_query("", cx);
                self.add_projects(&AddProjects, window, cx);
            }
            PaletteCommand::ChangeEditor => self.set_mode(Mode::Editors, cx),
            PaletteCommand::OpenConfig => {
                match open::open_project(&self.config, &store::config_path()) {
                    Ok(()) => window.remove_window(),
                    Err(err) => {
                        self.status = Some(format!("Could not open config: {err}").into());
                        cx.notify();
                    }
                }
            }
            PaletteCommand::Quit => cx.quit(),
        }
    }

    fn add_projects(&mut self, _: &AddProjects, window: &mut Window, cx: &mut Context<Self>) {
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add projects".into()),
        };
        self.pick(options, window, cx, |this, paths, _, cx| {
            this.add_paths(paths, cx)
        });
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut added = Vec::new();
        for path in paths {
            let Some(path) = store::normalize(&path.to_string_lossy()).filter(|p| p.is_dir())
            else {
                continue;
            };
            self.db.hidden.remove(&path);
            if !self.db.manual.contains(&path) {
                self.db.manual.push(path.clone());
            }
            added.push(path);
        }
        if added.is_empty() {
            return;
        }
        self.save(cx);
        self.reload_projects();
        if self.mode != Mode::Projects {
            self.set_mode(Mode::Projects, cx);
        } else {
            self.set_query("", cx);
        }
        // Put the newly added projects first so they can be opened right away.
        self.projects.sort_by_key(|p| !added.contains(&p.path));
        self.refilter(cx);
        self.status = Some(match added.as_slice() {
            [one] => format!("Added {}", store::display_path(one)).into(),
            many => format!("Added {} projects", many.len()).into(),
        });
    }

    fn choose_editor(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        match &self.editors[ix] {
            EditorOption::Detected(editor) => {
                let command = editor.command.clone();
                self.set_editor(command, window, cx);
            }
            EditorOption::FileManager => self.set_editor(String::new(), window, cx),
            EditorOption::Browse => {
                let options = PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some("Use as editor".into()),
                };
                self.pick(options, window, cx, |this, paths, window, cx| {
                    if let Some(path) = paths.into_iter().next() {
                        this.set_editor(path.to_string_lossy().into_owned(), window, cx);
                    }
                });
            }
        }
    }

    fn set_editor(&mut self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = store::set_editor(&command) {
            self.status = Some(format!("Could not save config: {err}").into());
            cx.notify();
            return;
        }
        self.config.editor = Some(command.clone());
        self.reload_projects();
        let label = self.name_of(&command);
        self.set_mode(Mode::Projects, cx);
        self.status = Some(format!("Projects will open in {label}").into());
        // First run: go straight on to picking projects.
        if self.projects.is_empty() {
            self.add_projects(&AddProjects, window, cx);
        }
    }

    /// Shows a native file dialog, keeping the palette open while it's up.
    fn pick(
        &mut self,
        options: PathPromptOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Vec<PathBuf>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.picking = true;
        let paths = cx.prompt_for_paths(options);
        cx.spawn_in(window, async move |this, cx| {
            let paths = paths
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .unwrap_or_default();
            this.update_in(cx, |this, window, cx| {
                this.picking = false;
                then(this, paths, window, cx);
                window.focus(&this.input.focus_handle(cx));
                #[cfg(windows)]
                crate::win::raise(window);
                #[cfg(not(windows))]
                window.activate_window();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        match self.list() {
            List::Commands => self.set_query("", cx),
            List::OpenWith => {
                let path = self.open_with.clone();
                self.set_mode(Mode::Projects, cx);
                self.select_where(|this, ix| Some(&this.projects[ix].path) == path.as_ref());
            }
            List::Editors if self.config.editor.is_some() => self.set_mode(Mode::Projects, cx),
            _ => window.remove_window(),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = store::save_db(&self.db) {
            self.status = Some(format!("Could not save: {err}").into());
            cx.notify();
        }
    }

    fn render_row(&self, row: usize, now: u64, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = &self.matches[row];
        let (title, subtitle) = self.item_text(m.ix);
        let meta = self.item_meta(m.ix, now);
        let highlight = HighlightStyle {
            color: Some(rgb(ACCENT).into()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let title_hl: Vec<_> = ranges(&title, &m.title_hl)
            .map(|r| (r, highlight))
            .collect();
        let subtitle_hl: Vec<_> = ranges(&subtitle, &m.subtitle_hl)
            .map(|r| (r, highlight))
            .collect();
        let pinned = self.list() == List::Projects && self.projects[m.ix].pinned;

        // uniform_list lays each row out on its own, so both levels need an explicit width.
        div().w_full().px_2().child(
            div()
                .id(row)
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_3()
                .rounded_md()
                .flex()
                .items_center()
                .gap_3()
                .when(row == self.selected, |d| d.bg(rgb(SELECTED)))
                .hover(|d| d.bg(rgb(SELECTED)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.selected = row;
                    this.confirm(&Confirm, window, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_sm()
                                .text_color(rgb(TEXT))
                                .when(pinned, |d| {
                                    d.child(div().size(px(6.)).rounded_full().bg(rgb(ACCENT)))
                                })
                                .child(StyledText::new(title).with_highlights(title_hl)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(StyledText::new(subtitle).with_highlights(subtitle_hl)),
                        ),
                )
                .when(meta.top.is_some() || meta.bottom.is_some(), |d| {
                    let line = |text: String, color: u32| {
                        div()
                            .text_color(rgb(color))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(text)
                    };
                    d.child(
                        div()
                            .flex_none()
                            .max_w(px(240.))
                            .flex()
                            .flex_col()
                            .items_end()
                            .text_xs()
                            .children(meta.top.map(|(text, color)| line(text, color)))
                            .children(meta.bottom.map(|text| line(text, MUTED))),
                    )
                }),
        )
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.add_candidate.is_some() {
            return div().flex_1().into_any_element();
        }
        if self.list() == List::Projects && self.projects.is_empty() {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("No projects yet")
                .child(
                    div()
                        .id("add-projects")
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(SELECTED))
                        .text_color(rgb(TEXT))
                        .cursor_pointer()
                        .hover(|d| d.bg(rgb(BORDER)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_projects(&AddProjects, window, cx)
                        }))
                        .child(format!("Add projects…  {}-o", secondary())),
                )
                .child("or paste a folder path above")
                .into_any_element();
        }
        div()
            .flex_1()
            .p_4()
            .text_sm()
            .text_color(rgb(MUTED))
            .child("No matches")
            .into_any_element()
    }

    fn render_banner(&self) -> Option<impl IntoElement + use<>> {
        let (title, lines): (String, Vec<String>) = match self.mode {
            Mode::Projects => return None,
            Mode::Editors => {
                let mut lines = vec![format!(
                    "The default for all projects. Change it any time with {}-e.",
                    secondary()
                )];
                if self.config.editor.is_none() {
                    lines.push(format!(
                        "proj keeps running in the background. Press {} to bring it up.",
                        self.config.hotkey
                    ));
                }
                ("Which editor should open your projects?".into(), lines)
            }
            Mode::OpenWith => {
                let name = self
                    .open_with_project()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let m = secondary();
                (
                    format!("Open {name} with…"),
                    vec![format!(
                        "{m}-s adds or removes an editor for this project; with two or more, \
                         enter asks which. {m}-↵ makes one its default."
                    )],
                )
            }
        };
        Some(
            div()
                .px_4()
                .pt_3()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(div().text_sm().text_color(rgb(TEXT)).child(title))
                .children(lines),
        )
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

/// Converts matched char byte offsets into merged byte ranges.
fn ranges<'a>(text: &'a str, offsets: &'a [usize]) -> impl Iterator<Item = Range<usize>> + 'a {
    let mut out: Vec<Range<usize>> = Vec::new();
    for &start in offsets {
        let end = start + text[start..].chars().next().map_or(1, char::len_utf8);
        match out.last_mut() {
            Some(last) if last.end == start => last.end = end,
            _ => out.push(start..end),
        }
    }
    out.into_iter()
}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = if !self.matches.is_empty() {
            let now = store::now();
            uniform_list(
                "items",
                self.matches.len(),
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    range.map(|row| this.render_row(row, now, cx)).collect()
                }),
            )
            .track_scroll(self.scroll.clone())
            .flex_1()
            .py_1()
            .into_any_element()
        } else {
            self.render_empty(cx)
        };

        let add_row = self.add_candidate.as_ref().map(|path| {
            div()
                .mx_2()
                .mt_1()
                .px_3()
                .h(px(ROW_HEIGHT))
                .rounded_md()
                .bg(rgb(SELECTED))
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .child(div().text_color(rgb(ACCENT)).child("Add project"))
                .child(div().text_color(rgb(TEXT)).child(store::display_path(path)))
                .child(div().ml_auto().text_xs().text_color(rgb(MUTED)).child("↵"))
        });

        let hint = |key: String, label: &'static str| {
            div()
                .flex()
                .gap_1()
                .child(div().text_color(rgb(TEXT)).child(key))
                .child(label)
        };
        let m = secondary();
        let hints: Vec<_> = match self.list() {
            List::Projects => vec![
                hint("↵".into(), "open"),
                hint("alt-↵".into(), "open with"),
                hint("shift-↵".into(), "terminal"),
                hint(format!("{m}-s"), "pin"),
                hint(">".into(), "commands"),
            ],
            List::OpenWith => vec![
                hint("↵".into(), "open"),
                hint(format!("{m}-s"), "add/remove"),
                hint(format!("{m}-↵"), "make default"),
                hint("esc".into(), "back"),
            ],
            List::Commands => vec![hint("↵".into(), "run"), hint("esc".into(), "back")],
            List::Editors => {
                let esc = if self.config.editor.is_some() {
                    "back"
                } else {
                    "close"
                };
                vec![hint("↵".into(), "select"), hint("esc".into(), esc)]
            }
        };
        let footer = div()
            .h(px(30.))
            .px_4()
            .border_t_1()
            .border_color(rgb(BORDER))
            .flex()
            .items_center()
            .gap_4()
            .text_xs()
            .text_color(rgb(MUTED))
            .child(match (&self.status, self.list()) {
                (Some(status), _) => div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(rgb(ACCENT))
                    .child(status.clone()),
                (None, List::Projects) => div().child(format!("{} projects", self.projects.len())),
                (None, _) => div(),
            })
            .child(div().flex_1())
            .children(hints);

        div()
            .key_context("Palette")
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.select(-1, cx)))
            .on_action(cx.listener(Self::confirm))
            .on_action(
                cx.listener(|this, _: &Reveal, window, cx| match this.list() {
                    List::OpenWith => this.make_project_default(cx),
                    _ => this.open_selected(Target::FileManager, window, cx),
                }),
            )
            .on_action(cx.listener(|this, _: &OpenTerminal, window, cx| {
                this.open_selected(Target::Terminal, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenWithMenu, _, cx| {
                if let Some(project) = this.selected_project() {
                    let path = project.path.clone();
                    this.show_open_with(path, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| match this.list() {
                List::OpenWith => this.toggle_project_editor(cx),
                _ => this.toggle_pin(cx),
            }))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::remove))
            .on_action(cx.listener(Self::add_projects))
            .on_action(
                cx.listener(|this, _: &ChooseEditor, _, cx| this.set_mode(Mode::Editors, cx)),
            )
            .on_action(cx.listener(Self::dismiss))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_lg()
            .overflow_hidden()
            .text_color(rgb(TEXT))
            .child(
                div()
                    .h(px(52.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .text_size(px(17.))
                    .line_height(px(24.))
                    .child(self.input.clone()),
            )
            .children(self.render_banner())
            .children(add_row)
            .child(list)
            .child(footer)
    }
}
