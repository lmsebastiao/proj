//! The window switcher: open editor windows, like Alt+Tab for your projects.
//! It stays up while the shortcut's modifiers are held and letting go switches.
//! Every open editor window has its own card, even for the same project.

use std::time::Duration;

use global_hotkey::hotkey::Modifiers;
use gpui::{
    AnyElement, Context, Focusable, KeyDownEvent, Keystroke, Pixels, Size, Window, div, prelude::*,
    px, rgb, size,
};

use crate::{
    platform,
    store::{self, Project},
    switcher::{self, EditorWindow},
};

use super::{
    Palette,
    items::Mode,
    keymap::{CloseWindow, Confirm, Dismiss},
    project_image::project_image,
    theme::{FONT_SIZE, icons},
    tooltip::tooltip,
};

pub(super) const CARD_WIDTH: f32 = 232.;
pub(super) const CARD_HEIGHT: f32 = 150.;

fn number_index(key: &str, count: usize) -> Option<usize> {
    let index = key.parse::<usize>().ok()?.checked_sub(1)?;
    (index < 9 && index < count).then_some(index)
}

pub(crate) fn switcher_size(count: usize) -> Size<Pixels> {
    let columns = count.clamp(1, 3);
    let rows = count.div_ceil(3).clamp(1, 3);
    let width = if count == 0 {
        420.
    } else {
        28. + columns as f32 * CARD_WIDTH + (columns - 1) as f32 * 12.
    };
    size(
        px(width),
        px(28. + rows as f32 * CARD_HEIGHT + (rows - 1) as f32 * 12.),
    )
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

    /// Alt+1…9 switches directly to that card, only while holding the switcher open.
    fn switch_to_number(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mods = keystroke.modifiers;
        if self.hold.is_none() || !mods.alt || mods.control || mods.platform || mods.shift {
            return false;
        }
        let Some(ix) = number_index(&keystroke.key, self.windows.len()) else {
            return false;
        };
        self.hold = None;
        self.switch_to(ix, window, cx);
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

                        let project = self.window_projects[ix]
                            .and_then(|project| self.projects.get(project))
                            .filter(|project| !project.is_workspace())
                            .map(|project| project.path.clone());
                        let icon = project_image(project, entry.exe.clone(), 40., window, cx);
                        div()
                            .id(("window-card", ix))
                            .w(px(CARD_WIDTH))
                            .h(px(CARD_HEIGHT))
                            .p_3()
                            .relative()
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
                            .children((ix < 9).then(|| {
                                div()
                                    .absolute()
                                    .bottom(px(8.))
                                    .right(px(9.))
                                    .text_size(px(12.))
                                    .text_color(rgb(t.muted))
                                    .child((ix + 1).to_string())
                            }))
                            .child(
                                div()
                                    .id(("close-window", ix))
                                    .absolute()
                                    .top(px(5.))
                                    .right(px(5.))
                                    .size(px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                                    .text_size(px(12.))
                                    .text_color(rgb(t.muted))
                                    .hover(|d| d.bg(rgb(t.border)).text_color(rgb(t.danger)))
                                    .cursor_pointer()
                                    .tooltip(tooltip("Close window · ctrl-w", t))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.close_row(ix, cx);
                                    }))
                                    .child(icons::CLOSE),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .h(px(24.))
                                    .pr(px(28.))
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
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .key_context("Palette")
            .track_focus(&focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.switch_to_number(&event.keystroke, window, cx) {
                    cx.stop_propagation();
                }
            }))
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
            .text_color(rgb(t.text))
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

#[cfg(test)]
mod tests {
    use super::number_index;

    #[test]
    fn number_picks_only_an_existing_card_among_the_first_nine() {
        assert_eq!(number_index("1", 3), Some(0));
        assert_eq!(number_index("3", 3), Some(2));
        assert_eq!(number_index("0", 3), None);
        assert_eq!(number_index("4", 3), None);
        assert_eq!(number_index("9", 10), Some(8));
        assert_eq!(number_index("10", 10), None);
        assert_eq!(number_index("x", 3), None);
    }
}
