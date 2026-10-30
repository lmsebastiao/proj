//! The window switcher: open editor windows, like Alt+Tab for your projects.
//! It stays up while the shortcut's modifiers are held and letting go switches.
//! Every open editor window has its own card, even for the same project.

use std::time::Duration;

use global_hotkey::hotkey::Modifiers;
use gpui::{AnyElement, Context, Focusable, Window, div, prelude::*, px, rgb};

use crate::{
    platform,
    store::{self, Project},
    switcher::{self, EditorWindow},
};

use super::{
    Palette,
    app_icon::app_icon,
    items::Mode,
    keymap::{CloseWindow, Confirm, Dismiss},
    theme::{FONT_SIZE, SMALL_FONT_SIZE, icons},
};

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
        this.selected = selected.min(this.matches.len().saturating_sub(1));

        this.wait_for_release(window, cx);
        this
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
                    if this.windows.is_empty() {
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

    /// Another press of the switcher's shortcut while it's showing: move forward
    /// (`delta` 1), or backward with shift (-1). Returns false when the palette isn't
    /// the switcher.
    pub fn cycle(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if self.mode != Mode::Switch {
            return false;
        }
        let len = self.windows.len();
        if len > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(len as isize) as usize;
            self.switch_scroll.scroll_to_item(self.selected / 3);
            cx.notify();
        }
        true
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

    /// The switcher's rows: one per editor window.
    pub(super) fn arrange_rows(&mut self) {
        self.switch_rows = switcher::rows(&self.windows);
    }

    /// Ctrl-W in the switcher: closes the highlighted row's window.
    pub(super) fn close_window(&mut self, _: &CloseWindow, _: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Switch {
            self.close_row(self.selected, cx);
        }
    }

    /// Asks the window of the switcher's `row` to close and takes it off the list.
    pub(super) fn close_row(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.mode != Mode::Switch {
            return;
        }
        let Some(&w) = self.switch_rows.get(row).and_then(|row| row.first()) else {
            return;
        };
        self.selected = row;
        let closed = self.windows.remove(w);
        platform::close_window(closed.window);
        self.match_windows();
        self.refilter(cx);
        self.selected = row.min(self.windows.len().saturating_sub(1));
        self.switch_scroll.scroll_to_item(self.selected / 3);
        self.notice(format!("Closing {}", closed.title), cx);
    }

    pub(super) fn render_switcher(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.theme;
        let focus = self.input.focus_handle(cx);
        if !focus.is_focused(window) && window.is_window_active() {
            window.focus(&focus);
        }
        let cards: Vec<AnyElement> = self
            .windows
            .chunks(3)
            .enumerate()
            .map(|(line, windows)| {
                div()
                    .flex()
                    .gap_3()
                    .children(windows.iter().enumerate().map(|(column, entry)| {
                        let ix = line * 3 + column;
                        let detail = self.window_projects[ix]
                            .map(|project| self.projects[project].name.clone())
                            .unwrap_or_else(|| entry.editor.clone());
                        let icon = if platform::HAS_APP_ICONS {
                            app_icon(Some(entry.exe.clone()), 40.).into_any_element()
                        } else {
                            div().child(icons::PROGRAM).into_any_element()
                        };
                        div()
                            .id(("window-card", ix))
                            .w(px(232.))
                            .h(px(114.))
                            .p_3()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .rounded_md()
                            .border_2()
                            .border_color(rgb(if ix == self.selected {
                                t.accent
                            } else {
                                t.border
                            }))
                            .bg(rgb(if ix == self.selected {
                                t.selected
                            } else {
                                t.hover
                            }))
                            .hover(|d| d.bg(rgb(t.selected)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.selected = ix;
                                this.switch_to(ix, window, cx);
                            }))
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(FONT_SIZE))
                                    .child(entry.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon),
                            )
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(SMALL_FONT_SIZE))
                                    .text_color(rgb(t.muted))
                                    .child(detail),
                            )
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .key_context("Palette")
            .track_focus(&focus)
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(|this, _: &Confirm, window, cx| {
                if !this.windows.is_empty() {
                    this.switch_to(this.selected, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Dismiss, window, _| {
                this.hold = None;
                window.remove_window();
            }))
            .on_action(cx.listener(|this, _: &super::keymap::SelectNext, _, cx| {
                this.cycle(1, cx);
            }))
            .on_action(cx.listener(|this, _: &super::keymap::SelectPrev, _, cx| {
                this.cycle(-1, cx);
            }))
            .on_action(cx.listener(|this, _: &super::keymap::MoveDown, _, cx| {
                this.cycle(1, cx);
            }))
            .on_action(cx.listener(|this, _: &super::keymap::MoveUp, _, cx| {
                this.cycle(-1, cx);
            }))
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_lg()
            .child(if self.windows.is_empty() {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(FONT_SIZE))
                    .text_color(rgb(t.muted))
                    .child(platform::window_access_hint().unwrap_or("No editor windows are open"))
                    .into_any_element()
            } else {
                div()
                    .id("switch-cards")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.switch_scroll)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(cards)
                    .into_any_element()
            })
            .into_any_element()
    }

    pub(super) fn window_choice_project(&self) -> Option<&Project> {
        let key = self.choosing_window.as_ref()?;
        self.projects.iter().find(|p| &p.key() == key)
    }

    /// Select a specific, confidently matched window or explicitly open anew.
    pub(super) fn choose_project_window(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(&w) = self.window_choices.get(ix) {
            self.switch_to(w, window, cx);
        } else if ix == self.window_choices.len()
            && let Some(project) = self.window_choice_project().cloned()
        {
            let editor = project.default_editor(&self.config);
            self.launch(&project, super::items::Target::Editor(editor), window, cx);
        }
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
