//! The window switcher: open editor windows, like Alt+Tab for your projects.
//! It stays up while the shortcut's modifiers are held and letting go switches;
//! typing while holding them keeps it up to search in instead. A project's
//! windows share a row, which → opens up.

use std::time::Duration;

use global_hotkey::hotkey::Modifiers;
use gpui::{Context, Keystroke, Pixels, SharedString, Window, div, prelude::*, px, rgb};

use crate::{
    launcher, platform,
    store::{self, Project},
    switcher::{self, EditorWindow},
};

use super::{
    Palette, SWITCH_PREFIX,
    items::{List, Mode},
    keymap::{CloseWindow, Confirm},
    theme::{FONT_SIZE, ROW_HEIGHT, Theme},
};

/// A switcher row being dragged to another place in the list, drawn under the
/// mouse as a copy of the row.
#[derive(Clone)]
pub(super) struct DraggedWindow {
    /// Its index in `Palette::switch_rows`.
    pub(super) ix: usize,
    pub(super) title: SharedString,
    pub(super) width: Pixels,
    pub(super) theme: Theme,
}

impl Render for DraggedWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .w(self.width)
            .h(px(ROW_HEIGHT))
            .px_3()
            .rounded_md()
            .flex()
            .items_center()
            .bg(rgb(t.selected))
            .border_1()
            .border_color(rgb(t.border))
            .shadow_lg()
            .text_size(px(FONT_SIZE))
            .text_color(rgb(t.text))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(self.title.clone()),
            )
    }
}

impl Palette {
    /// The switcher, with the row of `windows[selected]` highlighted, until
    /// `hold` is let go.
    pub fn switcher(
        window: &mut Window,
        cx: &mut Context<Self>,
        windows: Vec<EditorWindow>,
        selected: usize,
        hold: Modifiers,
    ) -> Self {
        let mut this = Self::new(window, cx, windows);
        this.hold = Some(hold);
        this.set_mode(Mode::Switch, cx);
        this.selected = this
            .switch_rows
            .iter()
            .position(|row| row.contains(&selected))
            .unwrap_or(0)
            .min(this.matches.len().saturating_sub(1));
        this.set_switch_placeholder(cx);
        if this.windows.is_empty() {
            let hint = platform::window_access_hint().unwrap_or("No editor windows are open");
            this.say(hint, cx);
        }
        this.wait_for_release(window, cx);
        this
    }

    /// Stops waiting for the modifiers to be let go: the switcher stays open
    /// to search in until enter or esc.
    fn keep_open(&mut self, cx: &mut Context<Self>) {
        self.hold = None;
        self.set_switch_placeholder(cx);
        cx.notify();
    }

    fn set_switch_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder = if self.hold.is_some() {
            "Let go to switch · type to search"
        } else {
            "Search open windows…"
        };
        self.input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }

    /// A key typed in the switcher. Typed with alt, as it is while holding the
    /// switcher open, it wouldn't reach the search box by itself, so it's typed
    /// here; the first one keeps the switcher open to search in. Returns
    /// whether the key was typed.
    pub(super) fn type_in_switcher(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.mode != Mode::Switch || (self.hold.is_none() && !keystroke.modifiers.alt) {
            return false;
        }
        // Not keys like "up" or "f1", nor space or tab, nor shortcuts with ctrl
        // (an AltGr character comes as one, but with its `key_char`).
        let mods = keystroke.modifiers;
        let text = keystroke
            .key_char
            .clone()
            .or_else(|| {
                (!mods.control && !mods.platform && keystroke.key.chars().count() == 1)
                    .then(|| keystroke.key.clone())
            })
            .filter(|text| text.chars().all(|c| !c.is_whitespace() && !c.is_control()));
        let Some(text) = text else {
            return false;
        };
        if self.hold.is_some() {
            self.keep_open(cx);
        }
        self.input
            .update(cx, |input, cx| input.insert(&text, window, cx));
        true
    }

    fn wait_for_release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Letting go of the modifier is a key event for whichever window has focus,
        // and can happen before this one gets it, so poll the keyboard instead.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(15))
                    .await;
                let done = this.update_in(cx, |this, window, cx| {
                    // Left the switcher some other way: letting go must not act
                    // on whatever list is showing now.
                    let (Some(mods), Mode::Switch) = (this.hold, this.mode) else {
                        this.hold = None;
                        return true;
                    };
                    if platform::modifiers_held(mods) {
                        return false;
                    }
                    // Let go while dragging a row: keep the list open to drop it.
                    if cx.has_active_drag() {
                        this.keep_open(cx);
                        return true;
                    }
                    this.hold = None;
                    if this.matches.is_empty() {
                        window.remove_window();
                    } else {
                        this.confirm(&Confirm, window, cx);
                    }
                    true
                });
                if done.unwrap_or(true) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Another press of the switcher's shortcut while it's showing: move down
    /// (`delta` 1), or up with shift (-1). Returns false when the palette isn't
    /// the switcher.
    pub fn cycle(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if self.mode != Mode::Switch {
            return false;
        }
        self.select(delta, cx);
        true
    }

    /// A key pressed while the switcher is held open: 1 to 9 switch straight to
    /// the row with that number. Returns whether the key was one of those.
    pub(super) fn switch_to_number(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.hold.is_none() || self.mode != Mode::Switch {
            return false;
        }
        let Some(n) = key.parse::<usize>().ok().filter(|n| (1..=9).contains(n)) else {
            return false;
        };
        // A number past the last row does nothing, rather than typing it.
        if let Some(&w) = self.switch_rows.get(n - 1).and_then(|row| row.first()) {
            self.hold = None;
            self.switch_to(w, window, cx);
        }
        true
    }

    /// Whether rows can be dragged to reorder the list: only while it shows
    /// every project's row in order, not search results or one project's windows.
    pub(super) fn can_reorder(&self) -> bool {
        self.list() == List::Switch && self.filter_query().is_empty() && self.expanded.is_none()
    }

    /// A row dropped on another: the row at `from` (all of a project's
    /// windows) takes the place of the one at `to`, and stays highlighted. The
    /// switcher keeps the new order.
    pub(super) fn move_window(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let len = self.switch_rows.len();
        if from == to || from >= len || to >= len || !self.can_reorder() {
            return;
        }
        let mut rows = self.switch_rows.clone();
        let moved = rows.remove(from);
        rows.insert(to, moved);
        self.windows = rows
            .iter()
            .flatten()
            .map(|&w| self.windows[w].clone())
            .collect();
        self.match_windows();
        launcher::set_switch_order(self.windows.iter().map(|w| w.window).collect(), cx);
        self.selected = to;
        cx.notify();
    }

    /// Works out which project each open window shows, and the rows.
    pub(super) fn match_windows(&mut self) {
        self.window_projects = self
            .windows
            .iter()
            .map(|w| w.project(&self.projects))
            .collect();
        self.open_keys = self
            .window_projects
            .iter()
            .flatten()
            .map(|&ix| self.projects[ix].key())
            .collect();
        self.arrange_rows();
    }

    /// The switcher's rows: a row per project, or `expanded`'s windows one by one.
    pub(super) fn arrange_rows(&mut self) {
        self.switch_rows = match self.expanded {
            Some(project) => (0..self.windows.len())
                .filter(|&w| self.window_projects[w] == Some(project))
                .map(|w| vec![w])
                .collect(),
            None => switcher::rows(&self.windows, &self.window_projects),
        };
    }

    /// →: a project row with several windows lists them one by one.
    /// Returns false when the highlighted row isn't one.
    pub(super) fn expand_row(&mut self, cx: &mut Context<Self>) -> bool {
        if self.expanded.is_some() {
            return false;
        }
        let Some(row) = self
            .matches
            .get(self.selected)
            .and_then(|m| self.switch_rows.get(m.ix))
            .filter(|row| row.len() > 1)
        else {
            return false;
        };
        self.expanded = self.window_projects[row[0]];
        self.arrange_rows();
        self.clear_switch_query(cx);
        true
    }

    /// ←: back from one project's windows to every project's rows, on its row.
    pub(super) fn collapse_row(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(project) = self.expanded.take() else {
            return false;
        };
        self.arrange_rows();
        self.clear_switch_query(cx);
        self.select_where(|this, ix| {
            this.window_projects[this.switch_rows[ix][0]] == Some(project)
        });
        cx.notify();
        true
    }

    /// An empty search, keeping the `@` that shows the windows in the project search.
    fn clear_switch_query(&mut self, cx: &mut Context<Self>) {
        let query = if self.mode == Mode::Projects {
            SWITCH_PREFIX.to_string()
        } else {
            String::new()
        };
        self.set_query(&query, cx);
    }

    /// Ctrl-W in the switcher: closes the highlighted row's window.
    pub(super) fn close_window(&mut self, _: &CloseWindow, _: &mut Window, cx: &mut Context<Self>) {
        self.close_row(self.selected, cx);
    }

    /// Asks the window of the switcher's `row` (for a project's row, the one
    /// it would switch to) to close, and takes it off the list.
    pub(super) fn close_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.list() != List::Switch {
            return;
        }
        let Some(&w) = self
            .matches
            .get(row)
            .and_then(|m| self.switch_rows.get(m.ix))
            .and_then(|row| row.first())
        else {
            return;
        };
        self.selected = row;
        let closed = self.windows.remove(w);
        platform::close_window(closed.window);
        self.match_windows();
        // Down to one window: no list of them to show.
        if let Some(project) = self.expanded
            && self
                .window_projects
                .iter()
                .filter(|&&p| p == Some(project))
                .count()
                < 2
        {
            self.expanded = None;
            self.arrange_rows();
        }
        let selected = self.selected;
        self.refilter(cx);
        self.selected = selected.min(self.matches.len().saturating_sub(1));
        self.notice(format!("Closing {}", closed.title), cx);
    }

    /// The open window showing `project`, preferring one of `editor`'s and then
    /// the one used last, as its index in `windows`.
    pub(super) fn open_window_of(&self, project: &Project, editor: &str) -> Option<usize> {
        let key = project.key();
        let editor = self.name_of(editor);
        (0..self.windows.len())
            .filter(|&w| self.window_projects[w].is_some_and(|p| self.projects[p].key() == key))
            .min_by_key(|&w| (self.windows[w].editor != editor, self.windows[w].rank()))
    }

    pub(super) fn switch_to(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Before closing, while proj is still in front and allowed to hand over focus.
        platform::focus_window(self.windows[ix].window);
        if let Some(project) = self.window_projects[ix] {
            store::record_open(&mut self.db, self.projects[project].key());
            self.save(cx);
        }
        window.remove_window();
    }
}
