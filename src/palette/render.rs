//! Drawing the palette.

use std::ops::Range;

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, StyledText, Window, div, prelude::*, px, rgb,
    uniform_list,
};

use crate::{paths, store};

use super::{Palette, items::*, keymap::*, secondary, theme::*};

impl Palette {
    pub(super) fn render_row(
        &self,
        row: usize,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
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
        // While marking, single projects get a numbered check box (the order is the
        // folder order in the workspace).
        let mark = (self.list() == List::Projects
            && !self.marked.is_empty()
            && !self.projects[m.ix].is_workspace())
        .then(|| {
            let position = self
                .marked
                .iter()
                .position(|p| p == &self.projects[m.ix].path);
            div()
                .flex_none()
                .size(px(16.))
                .rounded_sm()
                .border_1()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .map(|d| match position {
                    Some(i) => d
                        .bg(rgb(ACCENT))
                        .border_color(rgb(ACCENT))
                        .text_color(rgb(BG))
                        .child((i + 1).to_string()),
                    None => d.border_color(rgb(MUTED)),
                })
        });

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
                .children(mark)
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

    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn render_banner(&self) -> Option<impl IntoElement + use<>> {
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
                .child(div().text_color(rgb(TEXT)).child(paths::display_path(path)))
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
            List::Projects if !self.marked.is_empty() => vec![
                hint(
                    "↵".into(),
                    if self.marked.len() > 1 {
                        "open together"
                    } else {
                        "open"
                    },
                ),
                hint("alt-↵".into(), "open with"),
                hint("tab".into(), "mark"),
                hint("esc".into(), "clear"),
            ],
            List::Projects => vec![
                hint("↵".into(), "open"),
                hint("alt-↵".into(), "open with"),
                hint("tab".into(), "combine"),
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
                (None, List::Projects) if !self.marked.is_empty() => {
                    let names: Vec<String> = self
                        .marked
                        .iter()
                        .filter_map(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .collect();
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(rgb(ACCENT))
                        .child(names.join(" + "))
                }
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
                let entry = if this.marked.len() > 1 {
                    this.marked_workspace(cx)
                } else {
                    this.selected_project().cloned()
                };
                if let Some(entry) = entry {
                    this.show_open_with(entry.key(), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleMark, _, cx| this.toggle_mark(cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| match this.list() {
                List::OpenWith => this.toggle_project_editor(cx),
                _ => this.toggle_pin(cx),
            }))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::open_remote))
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
