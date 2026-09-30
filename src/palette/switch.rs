//! The window switcher: open editor windows, like Alt+Tab for your projects.
//! It stays up while the shortcut's modifiers are held and letting go switches;
//! opened with the search shortcut it stays up to type in instead.

use std::time::Duration;

use global_hotkey::hotkey::Modifiers;
use gpui::{Context, Pixels, SharedString, Window, div, prelude::*, px, rgb};

use crate::{
    launcher, platform, store,
    switcher::{self, EditorWindow},
};

use super::{
    Palette,
    items::{List, Mode},
    keymap::Confirm,
    theme::{FONT_SIZE, ROW_HEIGHT, Theme},
};

/// A switcher row being dragged to another place in the list, drawn under the
/// mouse as a copy of the row.
#[derive(Clone)]
pub(super) struct DraggedWindow {
    /// Its index in `Palette::windows`.
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
    /// The switcher, with `selected` highlighted: until `hold` is let go, or
    /// with `None`, for searching until enter or esc.
    pub fn switcher(
        window: &mut Window,
        cx: &mut Context<Self>,
        windows: Vec<EditorWindow>,
        selected: usize,
        hold: Option<Modifiers>,
    ) -> Self {
        let mut this = Self::new(window, cx, windows);
        this.hold = hold;
        this.set_mode(Mode::Switch, cx);
        this.selected = selected.min(this.matches.len().saturating_sub(1));
        this.set_switch_placeholder(cx);
        this.status = this
            .windows
            .is_empty()
            .then(|| "No editor windows are open".into());
        if hold.is_some() {
            this.wait_for_release(window, cx);
        }
        this
    }

    /// The search shortcut while the switcher is up: keep it open to type in.
    /// Returns false when the palette isn't the switcher.
    pub fn start_search(&mut self, cx: &mut Context<Self>) -> bool {
        if self.mode != Mode::Switch {
            return false;
        }
        self.hold = None;
        self.set_switch_placeholder(cx);
        cx.notify();
        true
    }

    fn set_switch_placeholder(&mut self, cx: &mut Context<Self>) {
        let placeholder = match (self.hold, self.config.switch_search_hotkey()) {
            (Some(_), Some(search)) => format!(
                "Let go to switch · {} to search",
                switcher::shortcut_label(&search)
            ),
            (Some(_), None) => "Let go to switch".into(),
            (None, _) => "Search open windows…".into(),
        };
        self.input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
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
                        this.start_search(cx);
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
    /// the window with that number. Returns whether the key was one of those.
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
        // A number past the last window does nothing, rather than typing it.
        if n <= self.windows.len() {
            self.hold = None;
            self.switch_to(n - 1, window, cx);
        }
        true
    }

    /// Whether rows can be dragged to reorder the list: only while it shows
    /// every window in order, not search results.
    pub(super) fn can_reorder(&self) -> bool {
        self.list() == List::Switch && self.filter_query().is_empty()
    }

    /// A row dropped on another: the window at `from` takes the place of the
    /// one at `to`, and stays highlighted. The switcher keeps the new order.
    pub(super) fn move_window(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let len = self.windows.len();
        if from == to || from >= len || to >= len || !self.can_reorder() {
            return;
        }
        let moved = self.windows.remove(from);
        self.windows.insert(to, moved);
        self.match_windows();
        launcher::set_switch_order(self.windows.iter().map(|w| w.window).collect(), cx);
        self.selected = to;
        cx.notify();
    }

    /// Works out which project each open window shows.
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
    }

    pub(super) fn switch_to(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Before closing, while proj is still in front and allowed to hand over focus.
        platform::focus_window(self.windows[ix].window);
        if let Some(project) = self.window_projects[ix] {
            self.db
                .opened
                .insert(self.projects[project].key(), store::now());
            self.save(cx);
        }
        window.remove_window();
    }
}
