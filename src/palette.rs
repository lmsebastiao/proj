//! The search dialog.

use std::{ops::Range, path::PathBuf};

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle,
    KeyBinding, PathPromptOptions, ScrollStrategy, SharedString, StyledText, Subscription,
    UniformListScrollHandle, Window, actions, div, prelude::*, px, rgb, uniform_list,
};

use crate::{
    fuzzy,
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
const SELECTED: u32 = 0x2c3038;
const ACCENT: u32 = 0x74ade8;
const ROW_HEIGHT: f32 = 46.;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Projects,
    Editors,
}

enum EditorOption {
    Detected(Editor),
    Browse,
    FileManager,
}

struct Match {
    ix: usize,
    title_hl: Vec<usize>,
    subtitle_hl: Vec<usize>,
}

pub struct Palette {
    input: Entity<TextInput>,
    mode: Mode,
    query: String,
    config: Config,
    db: Db,
    projects: Vec<Project>,
    editors: Vec<EditorOption>,
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
        this.projects = store::collect(&this.config, &this.db);
        let mode = if this.config.editor.is_none() { Mode::Editors } else { Mode::Projects };
        this.set_mode(mode, cx);
        this
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        if mode == Mode::Editors {
            self.editors = open::detect_editors()
                .into_iter()
                .map(EditorOption::Detected)
                .chain([EditorOption::Browse, EditorOption::FileManager])
                .collect();
        }
        let placeholder = match mode {
            Mode::Projects => "Search projects, or paste a folder path to add…",
            Mode::Editors => "Choose the editor to open projects with…",
        };
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        self.set_query("", cx);
    }

    fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.query = query.to_string();
        self.input.update(cx, |input, cx| input.set_text(query.to_string(), cx));
        self.refilter(cx);
    }

    /// Title and subtitle of an item in the current mode.
    fn item_text(&self, ix: usize) -> (String, String) {
        match self.mode {
            Mode::Projects => {
                let project = &self.projects[ix];
                (project.name.clone(), store::display_path(&project.path))
            }
            Mode::Editors => match &self.editors[ix] {
                EditorOption::Detected(editor) => {
                    let current = self.config.editor.as_deref() == Some(editor.command.as_str());
                    let title = if current { format!("{}  (current)", editor.name) } else { editor.name.to_string() };
                    (title, store::display_path(editor.command.as_ref()))
                }
                EditorOption::Browse => ("Other…".into(), "Pick any program".into()),
                EditorOption::FileManager => {
                    ("No editor".into(), "Open project folders in the file manager".into())
                }
            },
        }
    }

    fn item_count(&self) -> usize {
        match self.mode {
            Mode::Projects => self.projects.len(),
            Mode::Editors => self.editors.len(),
        }
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.clone();
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
                        // Small recency boost so frequently used projects win ties.
                        let recent = self.mode == Mode::Projects && self.projects[ix].last_opened > 0;
                        let score = score + 1000 + recent as i32 * 8;
                        return Some((score, Match { ix, title_hl: hl, subtitle_hl: Vec::new() }));
                    }
                    fuzzy::score(&query, &subtitle)
                        .map(|(score, hl)| (score, Match { ix, title_hl: Vec::new(), subtitle_hl: hl }))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.matches.extend(scored.into_iter().map(|(_, m)| m));
        }

        self.add_candidate = (self.mode == Mode::Projects && looks_like_path(&query))
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
        self.scroll.scroll_to_item(self.selected, ScrollStrategy::Center);
        cx.notify();
    }

    fn selected_project(&self) -> Option<&Project> {
        if self.mode != Mode::Projects {
            return None;
        }
        self.matches.get(self.selected).map(|m| &self.projects[m.ix])
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Editors => {
                if let Some(m) = self.matches.get(self.selected) {
                    self.choose_editor(m.ix, window, cx);
                }
            }
            Mode::Projects => match self.add_candidate.take() {
                Some(path) => self.add_paths(vec![path], cx),
                None => self.open_selected(false, window, cx),
            },
        }
    }

    fn reveal(&mut self, _: &Reveal, window: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(true, window, cx);
    }

    fn open_selected(&mut self, reveal: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.selected_project().cloned() else {
            return;
        };
        let result = if reveal {
            open::reveal(&project.path)
        } else {
            open::open_project(&self.config, &project.path)
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
                let editor = if reveal {
                    "the file manager".into()
                } else {
                    open::editor_label(self.config.editor.as_deref().unwrap_or(""))
                };
                self.status = Some(format!("Failed to launch {editor}: {err}").into());
                cx.notify();
            }
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
        self.db.opened.remove(project.path.to_string_lossy().as_ref());
        self.save(cx);
        self.projects.retain(|p| p.path != project.path);
        let selected = self.selected;
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.status = Some(format!("Removed {}", project.name).into());
    }

    fn add_projects(&mut self, _: &AddProjects, window: &mut Window, cx: &mut Context<Self>) {
        let options = PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add projects".into()),
        };
        self.pick(options, window, cx, |this, paths, _, cx| this.add_paths(paths, cx));
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut added = Vec::new();
        for path in paths {
            let Some(path) = store::normalize(&path.to_string_lossy()).filter(|p| p.is_dir()) else {
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
        self.projects = store::collect(&self.config, &self.db);
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
        let label = open::editor_label(&command);
        self.config.editor = Some(command);
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
            let paths = paths.await.ok().and_then(Result::ok).flatten().unwrap_or_default();
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
        if self.mode == Mode::Editors && self.config.editor.is_some() {
            self.set_mode(Mode::Projects, cx);
        } else {
            window.remove_window();
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = store::save_db(&self.db) {
            self.status = Some(format!("Could not save: {err}").into());
            cx.notify();
        }
    }

    fn render_row(&self, row: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = &self.matches[row];
        let (title, subtitle) = self.item_text(m.ix);
        let highlight = HighlightStyle {
            color: Some(rgb(ACCENT).into()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let title_hl: Vec<_> = ranges(&title, &m.title_hl).map(|r| (r, highlight)).collect();
        let subtitle_hl: Vec<_> = ranges(&subtitle, &m.subtitle_hl).map(|r| (r, highlight)).collect();

        // uniform_list lays each row out on its own, so both levels need an explicit width.
        div().w_full().px_2().child(
            div()
                .id(row)
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_3()
                .rounded_md()
                .flex()
                .flex_col()
                .justify_center()
                .when(row == self.selected, |d| d.bg(rgb(SELECTED)))
                .hover(|d| d.bg(rgb(SELECTED)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.selected = row;
                    this.confirm(&Confirm, window, cx);
                }))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(TEXT))
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
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.add_candidate.is_some() {
            return div().flex_1().into_any_element();
        }
        if self.mode == Mode::Projects && self.projects.is_empty() {
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
}

fn secondary() -> &'static str {
    if cfg!(target_os = "macos") { "cmd" } else { "ctrl" }
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
            uniform_list(
                "items",
                self.matches.len(),
                cx.processor(|this, range: Range<usize>, _, cx| {
                    range.map(|row| this.render_row(row, cx)).collect()
                }),
            )
            .track_scroll(self.scroll.clone())
            .flex_1()
            .py_1()
            .into_any_element()
        } else {
            self.render_empty(cx)
        };

        let banner = (self.mode == Mode::Editors).then(|| {
            let mut banner = div()
                .px_4()
                .pt_3()
                .pb_1()
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .text_color(rgb(MUTED))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(TEXT))
                        .child("Which editor should open your projects?"),
                )
                .child(format!("You can change this any time with {}-e.", secondary()));
            if self.config.editor.is_none() {
                banner = banner.child(format!(
                    "proj keeps running in the background. Press {} to bring it up.",
                    self.config.hotkey
                ));
            }
            banner
        });

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
        let mod_key = secondary();
        let hints: Vec<_> = match self.mode {
            Mode::Projects => vec![
                hint("↵".into(), "open"),
                hint(format!("{mod_key}-↵"), "folder"),
                hint(format!("{mod_key}-o"), "add"),
                hint(format!("{mod_key}-e"), "editor"),
                hint(format!("{mod_key}-d"), "remove"),
            ],
            Mode::Editors => {
                let esc = if self.config.editor.is_some() { "back" } else { "close" };
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
            .child(match (&self.status, self.mode) {
                (Some(status), _) => div().text_color(rgb(ACCENT)).child(status.clone()),
                (None, Mode::Projects) => div().child(format!("{} projects", self.projects.len())),
                (None, Mode::Editors) => div(),
            })
            .child(div().flex_1())
            .children(hints);

        div()
            .key_context("Palette")
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.select(-1, cx)))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::remove))
            .on_action(cx.listener(Self::add_projects))
            .on_action(cx.listener(|this, _: &ChooseEditor, _, cx| this.set_mode(Mode::Editors, cx)))
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
            .children(banner)
            .children(add_row)
            .child(list)
            .child(footer)
    }
}
