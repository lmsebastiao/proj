//! The window switcher: open editor windows, like Alt+Tab for your projects.
//! It stays up while the shortcut's modifiers are held and letting go switches;
//! opened with the search shortcut it stays up to type in instead.

use std::time::Duration;

use global_hotkey::hotkey::Modifiers;
use gpui::{Context, Window};

use crate::{
    platform, store,
    switcher::{self, EditorWindow},
};

use super::{Palette, items::Mode, keymap::Confirm};

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
