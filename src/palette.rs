//! The search dialog.

use std::{ops::Range, path::PathBuf};

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, FontWeight, HighlightStyle, KeyBinding,
    ScrollStrategy, SharedString, StyledText, Subscription, UniformListScrollHandle, Window,
    actions, div, prelude::*, px, rgb, uniform_list,
};

use crate::{
    fuzzy,
    input::{self, TextInput},
    open,
    store::{self, Config, Db, Project},
};

actions!(
    palette,
    [SelectNext, SelectPrev, Confirm, Reveal, Remove, Dismiss, QuitApp]
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

struct Match {
    ix: usize,
    name_hl: Vec<usize>,
    path_hl: Vec<usize>,
}

pub struct Palette {
    input: Entity<TextInput>,
    config: Config,
    db: Db,
    projects: Vec<Project>,
    matches: Vec<Match>,
    selected: usize,
    /// Set when the query is a path to an existing, not-yet-listed folder.
    add_candidate: Option<PathBuf>,
    status: Option<SharedString>,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search projects, or paste a folder path to add…", cx));
        let subscriptions = vec![
            cx.subscribe(&input, |this, _, _: &input::Changed, cx| {
                this.status = None;
                this.refilter(cx);
            }),
            // Behave like a launcher: clicking anywhere else dismisses it.
            cx.observe_window_activation(window, |_, window, _| {
                if !window.is_window_active() {
                    window.remove_window();
                }
            }),
        ];
        let mut this = Self {
            input,
            config: Config::default(),
            db: Db::default(),
            projects: Vec::new(),
            matches: Vec::new(),
            selected: 0,
            add_candidate: None,
            status: None,
            scroll: UniformListScrollHandle::new(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Re-reads config + database from disk and rescans folders, so edits made
    /// by hand or via the CLI show up on the next open.
    fn reload(&mut self, cx: &mut Context<Self>) {
        self.config = store::load_config();
        self.db = store::load_db();
        self.projects = store::collect(&self.config, &self.db);
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).text().trim().to_string();
        self.matches.clear();
        if query.is_empty() {
            self.matches.extend((0..self.projects.len()).map(|ix| Match {
                ix,
                name_hl: Vec::new(),
                path_hl: Vec::new(),
            }));
        } else {
            let mut scored: Vec<(i32, Match)> = self
                .projects
                .iter()
                .enumerate()
                .filter_map(|(ix, project)| {
                    if let Some((score, hl)) = fuzzy::score(&query, &project.name) {
                        // Small recency boost so frequently used projects win ties.
                        let recent = (project.last_opened > 0) as i32 * 8;
                        return Some((score + 1000 + recent, Match { ix, name_hl: hl, path_hl: Vec::new() }));
                    }
                    let path = store::display_path(&project.path);
                    fuzzy::score(&query, &path)
                        .map(|(score, hl)| (score, Match { ix, name_hl: Vec::new(), path_hl: hl }))
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            self.matches.extend(scored.into_iter().map(|(_, m)| m));
        }

        self.add_candidate = looks_like_path(&query)
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
        self.matches
            .get(self.selected)
            .map(|m| &self.projects[m.ix])
    }

    fn confirm(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.add_candidate.take() {
            self.db.hidden.remove(&path);
            if !self.db.manual.contains(&path) {
                self.db.manual.push(path.clone());
            }
            self.save(cx);
            self.status = Some(format!("Added {}", store::display_path(&path)).into());
            let status = self.status.clone();
            self.input.update(cx, |input, cx| input.set_text("", cx));
            self.projects = store::collect(&self.config, &self.db);
            self.refilter(cx);
            self.status = status;
            return;
        }
        self.open_selected(false, window, cx);
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
                let editor = if reveal { "file manager" } else { self.config.editor.as_str() };
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
        let ix = self.matches[self.selected].ix;
        self.projects.remove(ix);
        let selected = self.selected;
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.status = Some(format!("Removed {}", project.name).into());
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = store::save_db(&self.db) {
            self.status = Some(format!("Could not save: {err}").into());
            cx.notify();
        }
    }

    fn render_row(&self, row: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let m = &self.matches[row];
        let project = &self.projects[m.ix];
        let highlight = HighlightStyle {
            color: Some(rgb(ACCENT).into()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let name = StyledText::new(project.name.clone())
            .with_highlights(ranges(&project.name, &m.name_hl).map(|r| (r, highlight)));
        let path = store::display_path(&project.path);
        let path = StyledText::new(path.clone())
            .with_highlights(ranges(&path, &m.path_hl).map(|r| (r, highlight)));

        div()
            .id(row)
            .h(px(ROW_HEIGHT))
            .px_3()
            .mx_2()
            .rounded_md()
            .flex()
            .flex_col()
            .justify_center()
            .when(row == self.selected, |d| d.bg(rgb(SELECTED)))
            .hover(|d| d.bg(rgb(SELECTED)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected = row;
                this.open_selected(false, window, cx);
            }))
            .child(div().text_sm().text_color(rgb(TEXT)).child(name))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(path),
            )
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
            uniform_list(
                "projects",
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
            let message = if self.add_candidate.is_some() {
                String::new()
            } else if self.projects.is_empty() {
                format!(
                    "No projects yet. Paste a folder path to add one, or set scan_dirs in {}",
                    store::display_path(&store::config_path())
                )
            } else {
                "No matching projects".into()
            };
            div()
                .flex_1()
                .p_4()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(message)
                .into_any_element()
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

        let hint = |key: &'static str, label: &'static str| {
            div()
                .flex()
                .gap_1()
                .child(div().text_color(rgb(TEXT)).child(key))
                .child(label)
        };
        let secondary = if cfg!(target_os = "macos") { "⌘" } else { "ctrl" };
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
            .child(match &self.status {
                Some(status) => div().text_color(rgb(ACCENT)).child(status.clone()),
                None => div().child(format!("{} projects", self.projects.len())),
            })
            .child(div().flex_1())
            .child(hint("↵", "open"))
            .child(hint(if secondary == "⌘" { "⌘↵" } else { "ctrl-↵" }, "folder"))
            .child(hint(if secondary == "⌘" { "⌘D" } else { "ctrl-d" }, "remove"))
            .child(hint("esc", "close"));

        div()
            .key_context("Palette")
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.select(-1, cx)))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::reveal))
            .on_action(cx.listener(Self::remove))
            .on_action(|_: &Dismiss, window, _| window.remove_window())
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
            .children(add_row)
            .child(list)
            .child(footer)
    }
}
