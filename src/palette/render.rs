//! Drawing the palette.

use std::ops::Range;

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, StyledText, Window, div, prelude::*, px, rgb,
    uniform_list,
};

use crate::{input, paths, store};

use super::{Palette, items::*, keymap::*, secondary, shortcuts::Shortcut, theme::*};

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
        // Projects with an editor window open (the switcher lists those).
        let open =
            self.list() == List::Projects && self.open_keys.contains(&self.projects[m.ix].key());
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
                                .child(StyledText::new(title).with_highlights(title_hl))
                                .when(open, |d| {
                                    d.child(
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .border_1()
                                            .border_color(rgb(BORDER))
                                            .text_xs()
                                            .text_color(rgb(MUTED))
                                            .child("open"),
                                    )
                                }),
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
        if self.add_candidate.is_some()
            || self.clone_candidate.is_some()
            || self.list() == List::Rename
        {
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
            Mode::Projects | Mode::Switch => return None,
            Mode::Browse => (self.browse.as_ref()?.breadcrumb(), Vec::new()),
            Mode::Editors if self.config.editor.is_none() => (
                "Which editor should open your projects?".into(),
                vec![
                    "The default for all projects. Change it any time: type > and pick \
                     \"Change the default editor\"."
                        .into(),
                    format!(
                        "proj keeps running in the background. Press {} to bring it up.",
                        self.config.hotkey_label()
                    ),
                ],
            ),
            Mode::Editors => (
                "Default editor for all projects".into(),
                vec![
                    "↵ changes it for every project that doesn't have its own editor.".into(),
                    "Only for one project, once or as its default? Press alt-↵ on it instead."
                        .into(),
                ],
            ),
            Mode::Rename => {
                let project = self.renamed_project()?;
                let folders: Vec<String> = project
                    .paths()
                    .iter()
                    .filter_map(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .collect();
                (
                    format!("Rename {}", project.location()),
                    vec![format!(
                        "↵ saves. Search still finds it by {}.",
                        folders.join(" and ")
                    )],
                )
            }
            Mode::OpenWith => {
                let project = self.open_with_project()?;
                let name = &project.name;
                let m = secondary();
                let editor = self.name_of(&project.default_editor(&self.config));
                let now = match project.editor {
                    Some(_) => format!("{name}'s own default is {editor}."),
                    None => format!("{name} opens in the default editor, {editor}."),
                };
                (
                    format!("Open {name} with…"),
                    vec![
                        format!("{now} Here, ↵ opens it just this once and changes nothing."),
                        format!(
                            "{m}-↵ makes the highlighted editor {name}'s default \
                             (again: back to the default for all projects)."
                        ),
                    ],
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

        // A pasted folder path or git URL gets a row of its own above the matches.
        let action = self
            .add_candidate
            .as_ref()
            .map(|path| ("Add project", paths::display_path(path)))
            .or_else(|| {
                self.clone_candidate.as_ref().map(|target| {
                    let into = match &target.into {
                        Some(folder) => {
                            format!("{} into {}", target.name, paths::display_path(folder))
                        }
                        None => format!("{} into a folder you choose…", target.name),
                    };
                    ("Clone", into)
                })
            });
        let add_row = action.map(|(label, detail)| {
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
                .child(div().text_color(rgb(ACCENT)).child(label))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(rgb(TEXT))
                        .child(detail),
                )
                .child(div().ml_auto().text_xs().text_color(rgb(MUTED)).child("↵"))
        });

        let hint = |key: String, label: &'static str| {
            div()
                .flex()
                .gap_1()
                .child(div().text_color(rgb(TEXT)).child(key))
                .child(label)
        };
        let shortcuts = self.shortcuts();
        let mut hints: Vec<AnyElement> = shortcuts
            .iter()
            .filter_map(|s| Some(hint(s.keys[0].clone(), s.footer?).into_any_element()))
            .collect();
        // The rest are in the dropdown, opened by F1 or by clicking this.
        let more = shortcuts.iter().any(|s| s.footer.is_none()).then(|| {
            hint("f1".into(), "all keys")
                .id("all-keys")
                .cursor_pointer()
                .hover(|d| d.text_color(rgb(TEXT)))
                .when(self.show_shortcuts, |d| d.text_color(rgb(ACCENT)))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx)))
        });
        let dropdown =
            (self.show_shortcuts && more.is_some()).then(|| self.render_shortcuts(shortcuts, cx));
        if let Some(more) = more {
            hints.push(more.into_any_element());
        }
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
                (None, List::Browse) => div().child(format!("{} items", self.item_count())),
                (None, List::Switch) if self.hold.is_some() => div().child("let go to switch"),
                (None, List::Switch) => div().child(format!("{} windows", self.windows.len())),
                (None, _) => div(),
            })
            .child(div().flex_1())
            .children(hints);

        div()
            .key_context("Palette")
            // →/← browse into projects and folders, but only at the ends of the
            // search text, so they still move the cursor while editing it.
            .capture_action(cx.listener(|this, _: &input::Right, _, cx| {
                if this.input.read(cx).cursor_at_end() && this.enter(cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Left, _, cx| {
                if this.input.read(cx).cursor_at_start() && this.leave(cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.select(-1, cx)))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(|this, _: &ToggleProjectDefault, window, cx| {
                if this.list() == List::OpenWith {
                    this.toggle_project_default(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowInFileManager, window, cx| {
                this.open_selected(Target::FileManager, window, cx)
            }))
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
            .on_action(cx.listener(|this, _: &ToggleMark, _, cx| this.toggle_mark(1, cx)))
            .on_action(cx.listener(|this, _: &ToggleMarkUp, _, cx| this.toggle_mark(-1, cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin(cx)))
            .on_action(cx.listener(Self::rename))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::open_remote))
            .on_action(cx.listener(Self::remove))
            .on_action(cx.listener(Self::add_projects))
            .on_action(cx.listener(|this, _: &ToggleShortcuts, _, cx| this.toggle_shortcuts(cx)))
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
            .children(dropdown)
    }
}

impl Palette {
    fn toggle_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.show_shortcuts = !self.show_shortcuts;
        cx.notify();
    }

    /// Every shortcut of the current list, above the footer's "all keys".
    /// Clicking one closes the dropdown and runs it.
    fn render_shortcuts(
        &self,
        shortcuts: Vec<Shortcut>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let rows: Vec<_> = shortcuts
            .into_iter()
            .enumerate()
            .map(|(ix, shortcut)| {
                div()
                    .id(("shortcut", ix))
                    .mx_1()
                    .px_2()
                    .h(px(24.))
                    .flex_none()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(130.))
                            .flex_none()
                            .text_color(rgb(TEXT))
                            .child(shortcut.keys.join("  ")),
                    )
                    .child(div().text_color(rgb(MUTED)).child(shortcut.action))
                    .when_some(shortcut.run, |row, run| {
                        row.cursor_pointer()
                            .hover(|d| d.bg(rgb(SELECTED)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_shortcuts = false;
                                cx.notify();
                                window.dispatch_action(run.boxed_clone(), cx);
                            }))
                    })
            })
            .collect();
        div()
            .id("shortcuts")
            .absolute()
            .right(px(8.))
            .bottom(px(34.))
            .w(px(380.))
            .max_h(px(340.))
            .overflow_y_scroll()
            .occlude()
            .py_1()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_md()
            .shadow_lg()
            .text_xs()
            .children(rows)
    }
}
