//! Drawing the palette.

use std::{cmp::Ordering, ops::Range};

use gpui::{
    AnyElement, Context, FontWeight, HighlightStyle, KeyDownEvent, ModifiersChangedEvent,
    StyledText, Window, div, prelude::*, px, rgb, rgba, uniform_list,
};

use crate::{input, paths, store};

use super::{
    Palette,
    actions::{ActionEntry, ROW_ICONS, RowIcon},
    app_icon::{self, app_icon},
    items::*,
    keymap::*,
    secondary,
    shortcuts::Shortcut,
    switch::DraggedWindow,
    theme::*,
    tooltip::tooltip,
};

/// The group of a list row, so its icons can show while the mouse is over it.
const ROW_GROUP: &str = "row";

impl Palette {
    pub(super) fn render_row(
        &self,
        row: usize,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = self.theme;
        let m = &self.matches[row];
        let (title, subtitle) = self.item_text(m.ix);
        let meta = self.item_meta(m.ix, now);
        let highlight = HighlightStyle {
            color: Some(rgb(t.accent).into()),
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
                .size(px(18.))
                .rounded_sm()
                .border_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(SMALL_FONT_SIZE))
                .map(|d| match position {
                    Some(i) => d
                        .bg(rgb(t.accent))
                        .border_color(rgb(t.accent))
                        .text_color(rgb(t.bg))
                        .child((i + 1).to_string()),
                    None => d.border_color(rgb(t.muted)),
                })
        });
        // A switcher row's number, for the number keys while holding the
        // switcher and the switch-by-number shortcuts (alt+shift+1…9); and
        // while ctrl is held, a project's place for ctrl+1…9. The ones past 9
        // keep the space so the titles line up.
        let number = if self.list() == List::Switch
            && (self.hold.is_some() || self.config.switch_number_modifiers().is_some())
        {
            Some(m.ix)
        } else if self.list() == List::Projects && self.numbers_shown {
            Some(row)
        } else {
            None
        }
        .map(|n| {
            div()
                .flex_none()
                .w(px(12.))
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.muted))
                .when(n < 9, |d| d.child((n + 1).to_string()))
        });
        // Switcher rows can be dragged to another place in the list (it shows
        // every row in order then, so `row` is also the row's index).
        let dragged = self.can_reorder().then(|| DraggedWindow {
            ix: m.ix,
            title: title.clone().into(),
            width: px(0.),
            theme: t,
        });
        // The editor's or window's program icon. "Other…" and "No editor" get
        // none, but keep the space.
        let program = match self.list() {
            List::Editors | List::OpenWith => Some(match &self.editors[m.ix] {
                EditorOption::Detected(editor) => editor.app.clone(),
                EditorOption::Browse | EditorOption::FileManager => None,
            }),
            List::Switch => Some(Some(self.windows[self.switch_rows[m.ix][0]].exe.clone())),
            _ => None,
        }
        .map(|program| app_icon(program, app_icon::ROW_SIZE));
        // A project with its own editor: that editor's icon, before its name
        // on the right.
        let own_editor = (self.list() == List::Projects && !self.projects[m.ix].missing)
            .then(|| self.projects[m.ix].editor.as_ref())
            .flatten()
            .and_then(|command| self.apps.get(command).cloned().flatten());
        // A project whose folder is gone is dimmed; tags show after the name.
        let project = (self.list() == List::Projects).then(|| &self.projects[m.ix]);
        let missing = project.is_some_and(|p| p.missing);
        // Each tag in its own colour, so the same tag looks the same everywhere.
        let tags: Vec<_> = project
            .map(|p| p.tags.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|tag| {
                let color = t.tag_color(&tag);
                div()
                    .flex_none()
                    .px_1()
                    .rounded_sm()
                    .border_1()
                    // The colour, faded.
                    .border_color(rgba((color << 8) | 0x66))
                    .text_size(px(SMALL_FONT_SIZE))
                    .text_color(rgb(color))
                    .child(format!("#{tag}"))
            })
            .collect();
        // A switcher row's close button, like ctrl-w: shown on the highlighted
        // row, and on any row the mouse is over.
        let close = (self.list() == List::Switch).then(|| {
            div()
                .id(("close", row))
                .flex_none()
                .size(px(28.))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                .text_size(px(12.))
                .text_color(rgb(t.muted))
                .hover(|d| d.bg(rgb(t.border)).text_color(rgb(t.danger)))
                .tooltip(tooltip(format!("Close the window · {}-w", secondary()), t))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    // Not also a click on the row, which switches to it.
                    cx.stop_propagation();
                    this.close_row(row, cx);
                }))
                .when(row != self.selected, |d| {
                    d.invisible().group_hover(ROW_GROUP, |s| s.visible())
                })
                .child(icons::CLOSE)
        });
        // Pin, rename, remove and the actions menu: shown on the highlighted row,
        // and on any row the mouse is over.
        let icons = (self.list() == List::Projects).then(|| {
            let project = &self.projects[m.ix];
            let armed = self.confirm_remove.as_ref() == Some(&project.key());
            div()
                .flex()
                .flex_none()
                .gap_1()
                .relative()
                .children(ROW_ICONS.map(|icon| {
                    let (glyph, color) = match icon {
                        RowIcon::Pin if pinned => (icons::PINNED, t.accent),
                        RowIcon::Pin => (icons::PIN, t.muted),
                        RowIcon::Rename => (icons::RENAME, t.muted),
                        RowIcon::Remove => (icons::REMOVE, if armed { t.danger } else { t.muted }),
                        RowIcon::More => (icons::MORE, t.muted),
                    };
                    let tip = match icon {
                        RowIcon::Pin if pinned => "Unpin: back into the recent order".to_string(),
                        RowIcon::Pin => "Pin: keep it at the top".into(),
                        RowIcon::Rename => "Rename… (search still finds it by its folder)".into(),
                        RowIcon::Remove if armed => "Click again to remove it".into(),
                        RowIcon::Remove => "Remove from the list (the folder stays)".into(),
                        RowIcon::More => format!("All actions · {}-k", secondary()),
                    };
                    div()
                        .id((icon.id(), row))
                        .tooltip(tooltip(tip, t))
                        .size(px(28.))
                        .rounded_md()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                        .text_size(px(14.))
                        .text_color(rgb(color))
                        .hover(|d| d.bg(rgb(t.border)).text_color(rgb(t.text)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // Not also a click on the row, which opens the project.
                            cx.stop_propagation();
                            this.click_row_icon(row, icon, cx);
                        }))
                        .when(row != self.selected, |d| {
                            d.invisible().group_hover(ROW_GROUP, |s| s.visible())
                        })
                        .child(glyph)
                }))
                // While the icons are hidden, a pinned row still shows its pin, at the
                // end, wherever the pin icon sits among them.
                .when(pinned && row != self.selected, |d| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                            .text_size(px(14.))
                            .text_color(rgb(t.accent))
                            .group_hover(ROW_GROUP, |s| s.invisible())
                            .child(icons::PINNED),
                    )
                })
        });

        // A line between the pinned projects and the rest, while the list
        // shows them all in order.
        let after_pins = self.list() == List::Projects
            && self.filter_query().is_empty()
            && row > 0
            && !pinned
            && self.projects[self.matches[row - 1].ix].pinned;

        // uniform_list lays each row out on its own, so both levels need an explicit width.
        div()
            .w_full()
            .px_2()
            .relative()
            .when(after_pins, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(20.))
                        .right(px(20.))
                        .h(px(1.))
                        .bg(rgb(t.border)),
                )
            })
            .child(
                div()
                    .id(row)
                    .group(ROW_GROUP)
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px_3()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(row == self.selected, |d| d.bg(rgb(t.selected)))
                    .hover(|d| d.bg(rgb(t.selected)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected = row;
                        this.confirm(&Confirm, window, cx);
                    }))
                    .when_some(dragged, |d, dragged| {
                        d.on_drag(dragged, |dragged, _, window, cx| {
                            let width = window.viewport_size().width - px(18.);
                            cx.new(|_| DraggedWindow {
                                width,
                                ..dragged.clone()
                            })
                        })
                        // A line where it would land: above this row when coming
                        // from below, under it when coming from above.
                        .drag_over::<DraggedWindow>(move |style, dragged, _, _| {
                            match dragged.ix.cmp(&row) {
                                Ordering::Greater => style.border_t_2().border_color(rgb(t.accent)),
                                Ordering::Less => style.border_b_2().border_color(rgb(t.accent)),
                                Ordering::Equal => style,
                            }
                        })
                        .on_drop(cx.listener(
                            move |this, dragged: &DraggedWindow, _, cx| {
                                this.move_window(dragged.ix, row, cx);
                            },
                        ))
                    })
                    .children(number)
                    .children(mark)
                    .children(program)
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
                                    .text_size(px(FONT_SIZE))
                                    .text_color(rgb(if missing { t.muted } else { t.text }))
                                    .overflow_hidden()
                                    .child(StyledText::new(title).with_highlights(title_hl))
                                    .when(open, |d| {
                                        d.child(
                                            div()
                                                .id(("open", row))
                                                .flex_none()
                                                .size(px(8.))
                                                .rounded_full()
                                                .bg(rgb(t.open))
                                                .tooltip(tooltip(
                                                    "An editor window is open · ↵ switches to it",
                                                    t,
                                                )),
                                        )
                                    })
                                    .children(tags),
                            )
                            .child(
                                div()
                                    .text_size(px(SMALL_FONT_SIZE))
                                    .text_color(rgb(t.muted))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(StyledText::new(subtitle).with_highlights(subtitle_hl)),
                            ),
                    )
                    .children(icons)
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
                                .id(("meta", row))
                                .flex_none()
                                .max_w(px(280.))
                                .flex()
                                .flex_col()
                                .items_end()
                                .text_size(px(SMALL_FONT_SIZE))
                                .children(meta.top.map(|(text, color)| line(text, color)))
                                .children(meta.bottom.map(|text| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .children(own_editor.map(|program| {
                                            app_icon(Some(program), app_icon::SMALL_SIZE)
                                        }))
                                        .child(line(text, t.muted))
                                }))
                                // What the branch's marks, "missing" or "· →" mean.
                                .when_some(meta.tip, |d, tip| d.tooltip(tooltip(tip, t))),
                        )
                    })
                    .children(close),
            )
    }

    /// The actions menu: one-line rows with an icon, under section headings,
    /// and the highlighted row's explanation under it. A plain list rather
    /// than a `uniform_list`, as the headings make rows differ in height.
    fn render_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let pinned = self.actions_project().is_some_and(|p| p.pinned);
        let highlight = HighlightStyle {
            color: Some(rgb(t.accent).into()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let entries: Vec<AnyElement> = self
            .action_entries()
            .into_iter()
            .map(|entry| match entry {
                ActionEntry::Heading(label) => div()
                    .px_5()
                    .pt_2()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(rgb(t.muted))
                    .child(label.to_uppercase())
                    .into_any_element(),
                ActionEntry::Line => div()
                    .mx_4()
                    .my_2()
                    .h(px(1.))
                    .bg(rgb(t.border))
                    .into_any_element(),
                ActionEntry::Row(row) => {
                    let m = &self.matches[row];
                    let action = self.actions[m.ix];
                    let (title, subtitle) = self.item_text(m.ix);
                    let meta = self.item_meta(m.ix, 0);
                    let selected = row == self.selected;
                    // Remove is in the warning colour, icon and all.
                    let danger = action == ProjectAction::Remove;
                    let (color, icon_color) = if danger {
                        (t.danger, t.danger)
                    } else {
                        (t.text, t.muted)
                    };
                    let title_hl: Vec<_> = ranges(&title, &m.title_hl)
                        .map(|r| (r, highlight))
                        .collect();
                    div()
                        .id(("action", row))
                        .mx_2()
                        .px_3()
                        .rounded_md()
                        .flex()
                        .flex_col()
                        .when(selected, |d| d.bg(rgb(t.selected)))
                        .hover(|d| d.bg(rgb(t.selected)))
                        // What it does, when the mouse rests on it.
                        .when(!subtitle.is_empty(), |d| d.tooltip(tooltip(subtitle, t)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.selected = row;
                            this.confirm(&Confirm, window, cx);
                        }))
                        .child(
                            div()
                                .h(px(30.))
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_none()
                                        .w(px(18.))
                                        .flex()
                                        .justify_center()
                                        .when(!icons::FONT.is_empty(), |d| {
                                            d.font_family(icons::FONT)
                                        })
                                        .text_size(px(14.))
                                        .text_color(rgb(icon_color))
                                        .child(action.icon(pinned)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(px(FONT_SIZE - 1.))
                                        .text_color(rgb(color))
                                        .child(StyledText::new(title).with_highlights(title_hl)),
                                )
                                .children(meta.top.map(|(text, color)| {
                                    div()
                                        .flex_none()
                                        .max_w(px(260.))
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(px(SMALL_FONT_SIZE))
                                        .text_color(rgb(color))
                                        .child(text)
                                })),
                        )
                        .into_any_element()
                }
            })
            .collect();
        div()
            .id("actions")
            .flex_1()
            .overflow_y_scroll()
            .track_scroll(&self.actions_scroll)
            .py_1()
            .flex()
            .flex_col()
            .children(entries)
            .into_any_element()
    }

    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        if self.add_candidate.is_some()
            || self.clone_candidate.is_some()
            || self.list() == List::Text
        {
            return div().flex_1().into_any_element();
        }
        if self.list() == List::Templates && self.templates.is_empty() {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .px_6()
                .text_size(px(FONT_SIZE))
                .text_color(rgb(t.muted))
                .child("No templates yet")
                .child(
                    div()
                        .text_size(px(SMALL_FONT_SIZE))
                        .child("List folders and git URLs under templates in the config file (> Open config file)."),
                )
                .child(div().text_size(px(SMALL_FONT_SIZE)).child(format!(
                    "Any project works too: {}-k on it, then \"New project from this one\".",
                    secondary()
                )))
                .into_any_element();
        }
        if self.list() == List::Projects && self.projects.is_empty() {
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .text_size(px(FONT_SIZE))
                .text_color(rgb(t.muted))
                .child("No projects yet")
                .child(
                    div()
                        .id("add-projects")
                        .px_4()
                        .py_2()
                        .rounded_md()
                        .bg(rgb(t.selected))
                        .text_color(rgb(t.text))
                        .cursor_pointer()
                        .hover(|d| d.bg(rgb(t.border)))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_projects(&AddProjects, window, cx)
                        }))
                        .child(format!("Add projects…  {}-o", secondary())),
                )
                .child("or paste a folder path above")
                .into_any_element();
        }
        let empty = if self.list() == List::Switch && self.windows.is_empty() {
            crate::platform::window_access_hint().unwrap_or("No editor windows are open")
        } else {
            "No matches"
        };
        div()
            .flex_1()
            .p_4()
            .text_size(px(FONT_SIZE))
            .text_color(rgb(t.muted))
            .child(empty)
            .into_any_element()
    }

    pub(super) fn render_banner(&self) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let (title, lines): (String, Vec<String>) = match self.mode {
            // One project's windows, after → on its row.
            _ if self.list() == List::Switch && self.expanded.is_some() => {
                let project = &self.projects[self.expanded?];
                (
                    format!("{}'s windows", project.name),
                    vec!["← back to every project's".into()],
                )
            }
            Mode::Projects | Mode::Switch | Mode::Actions => return None,
            Mode::Tags => {
                let project = self.edited_project()?;
                (
                    format!("Tags for {}", project.name),
                    vec![
                        "Words, separated by spaces or commas. ↵ saves.".into(),
                        "Then search for #tag to list just the projects with it.".into(),
                    ],
                )
            }
            Mode::AddCommand => {
                let project = self.edited_project()?;
                (
                    format!("Add a command to {}", project.name),
                    vec![format!(
                        "↵ adds it to its actions ({}-k), to run in a terminal in {}.",
                        secondary(),
                        project.location()
                    )],
                )
            }
            Mode::Templates => (
                "New project from a template".into(),
                vec!["↵ picks one, then you name the new project.".into()],
            ),
            Mode::NewProject => {
                let template = self.new_from.as_ref()?;
                let into = match self.config.scan_dirs.first() {
                    Some(dir) => format!("in {}", paths::display_path(dir)),
                    None => "in a folder you pick next".into(),
                };
                (
                    format!("New project from {}", template.name()),
                    vec![format!(
                        "↵ makes it {into}, with a git history of its own, and opens it."
                    )],
                )
            }
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
                let project = self.edited_project()?;
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
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.muted))
                .child(
                    div()
                        .text_size(px(FONT_SIZE))
                        .text_color(rgb(t.text))
                        .child(title),
                )
                .children(lines),
        )
    }

    /// Which project the actions menu is for: one line with a rule under it, so
    /// it doesn't read as the first action.
    pub(super) fn render_actions_header(&self) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let project = self
            .actions_project()
            .filter(|_| self.mode == Mode::Actions)?;
        Some(
            div()
                .mx_4()
                .pt_3()
                .pb_2()
                .mb_1()
                .border_b_1()
                .border_color(rgb(t.border))
                .flex()
                .items_baseline()
                .gap_2()
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.muted))
                .child(div().flex_none().child("Actions for"))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(FONT_SIZE))
                        .text_color(rgb(t.text))
                        .child(project.name.clone()),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(project.location()),
                ),
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
        let t = self.theme;
        let list = if self.list() == List::Actions && !self.matches.is_empty() {
            self.render_actions(cx)
        } else if !self.matches.is_empty() {
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
                .bg(rgb(t.selected))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(FONT_SIZE))
                .child(div().text_color(rgb(t.accent)).child(label))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(rgb(t.text))
                        .child(detail),
                )
                .child(
                    div()
                        .ml_auto()
                        .text_size(px(SMALL_FONT_SIZE))
                        .text_color(rgb(t.muted))
                        .child("↵"),
                )
        });

        let hint = |key: String, label: &'static str| {
            div()
                .flex()
                .gap_1()
                .child(div().text_color(rgb(t.text)).child(key))
                .child(label)
        };
        let shortcuts = self.shortcuts();
        // Each says in full what it does when the mouse rests on it.
        let mut hints: Vec<AnyElement> = shortcuts
            .iter()
            .enumerate()
            .filter_map(|(ix, s)| {
                Some(
                    hint(s.keys[0].clone(), s.footer?)
                        .id(("hint", ix))
                        .tooltip(tooltip(s.action, t))
                        .into_any_element(),
                )
            })
            .collect();
        // The rest are in the dropdown, opened by F1 or by clicking this.
        let more = shortcuts.iter().any(|s| s.footer.is_none()).then(|| {
            hint("f1".into(), "all keys")
                .id("all-keys")
                .tooltip(tooltip("Every key of this list; click one to run it", t))
                .cursor_pointer()
                .hover(|d| d.text_color(rgb(t.text)))
                .when(self.show_shortcuts, |d| d.text_color(rgb(t.accent)))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx)))
        });
        let dropdown =
            (self.show_shortcuts && more.is_some()).then(|| self.render_shortcuts(shortcuts, cx));
        if let Some(more) = more {
            hints.push(more.into_any_element());
        }
        let footer = div()
            .h(px(FOOTER_HEIGHT))
            .px_4()
            .border_t_1()
            .border_color(rgb(t.border))
            .flex()
            .items_center()
            .gap_4()
            .text_size(px(SMALL_FONT_SIZE))
            .text_color(rgb(t.muted))
            .child(match (&self.status, self.list()) {
                (Some(status), _) => div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(rgb(t.accent))
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
                        .text_color(rgb(t.accent))
                        .child(names.join(" + "))
                }
                (None, List::Projects) if !self.filter_query().is_empty() => div().child(format!(
                    "{} of {} projects",
                    self.matches.len(),
                    self.projects.len()
                )),
                (None, List::Projects) => div().child(format!("{} projects", self.projects.len())),
                (None, List::Browse) => div().child(format!("{} items", self.item_count())),
                (None, List::Switch) if self.hold.is_some() => div().child("let go to switch"),
                (None, List::Switch) => div().child(format!("{} windows", self.windows.len())),
                // What the highlighted action does, as its tooltip says for the mouse.
                (None, List::Actions) => div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .children(
                        self.matches
                            .get(self.selected)
                            .map(|m| self.item_text(m.ix).1),
                    ),
                (None, _) => div(),
            })
            .child(div().flex_1())
            .children(hints);

        div()
            .key_context("Palette")
            // While the switcher is held open, a number switches to that window
            // and other keys start a search.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.switch_to_number(&event.keystroke.key, window, cx)
                    || this.open_number(&event.keystroke, window, cx)
                    || this.type_in_switcher(&event.keystroke, window, cx)
                {
                    cx.stop_propagation();
                }
            }))
            // Ctrl (cmd) alone held: the projects show the numbers ctrl+1…9 open.
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                let m = event.modifiers;
                this.ctrl_held(m.secondary() && !m.alt && !m.shift, cx);
            }))
            // Home/end move the text cursor; with nothing typed, the selection.
            .capture_action(cx.listener(|this, _: &input::Home, _, cx| {
                if this.query.is_empty() && !this.matches.is_empty() {
                    this.select_row(0, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::End, _, cx| {
                if this.query.is_empty() && !this.matches.is_empty() {
                    this.select_row(usize::MAX, cx);
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| this.select_page(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| this.select_page(-1, cx)))
            .on_action(cx.listener(Self::undo_remove))
            // →/← browse into projects and folders, but only at the ends of the
            // search text, so they still move the cursor while editing it.
            .capture_action(cx.listener(|this, _: &input::Right, _, cx| {
                if this.input.read(cx).cursor_at_end() && this.enter(cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Left, _, cx| {
                // Out of a project's windows also from just after the `@` that
                // lists them in the project search.
                let at_start = this.input.read(cx).cursor_at_start()
                    || (this.list() == List::Switch && this.filter_query().is_empty());
                if at_start && this.leave(cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(Self::remove_task))
            .on_action(cx.listener(Self::close_window))
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
            .on_action(cx.listener(|this, _: &ShowActions, _, cx| this.show_actions(cx)))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::open_remote))
            .on_action(cx.listener(Self::add_projects))
            .on_action(cx.listener(|this, _: &ToggleShortcuts, _, cx| this.toggle_shortcuts(cx)))
            .on_action(cx.listener(Self::dismiss))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_lg()
            .overflow_hidden()
            .text_color(rgb(t.text))
            .child(
                div()
                    .h(px(58.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(t.border))
                    .text_size(px(INPUT_FONT_SIZE))
                    .line_height(px(27.))
                    .child(self.input.clone()),
            )
            .children(self.render_banner())
            .children(self.render_actions_header())
            .children(add_row)
            .child(list)
            .child(footer)
            .children(dropdown)
    }
}

impl Palette {
    fn toggle_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.show_shortcuts = !self.show_shortcuts;
        // While it's open the arrows move through it, from its first key.
        self.shortcut_selected = self
            .shortcuts()
            .iter()
            .position(|s| s.run.is_some())
            .unwrap_or(0);
        self.shortcuts_scroll.scroll_to_item(self.shortcut_selected);
        cx.notify();
    }

    /// ↑/↓ with the dropdown open: the next key in it that can be run.
    pub(super) fn select_shortcut(&mut self, delta: isize, cx: &mut Context<Self>) {
        let runnable: Vec<usize> = self
            .shortcuts()
            .iter()
            .enumerate()
            .filter(|(_, s)| s.run.is_some())
            .map(|(ix, _)| ix)
            .collect();
        if runnable.is_empty() {
            return;
        }
        let at = runnable
            .iter()
            .position(|&ix| ix == self.shortcut_selected)
            .unwrap_or(0) as isize;
        self.shortcut_selected =
            runnable[(at + delta).rem_euclid(runnable.len() as isize) as usize];
        self.shortcuts_scroll.scroll_to_item(self.shortcut_selected);
        cx.notify();
    }

    /// Enter with the dropdown open: closes it and runs the highlighted key,
    /// like clicking it.
    pub(super) fn run_shortcut(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_shortcuts = false;
        cx.notify();
        let run = self
            .shortcuts()
            .into_iter()
            .nth(self.shortcut_selected)
            .and_then(|s| s.run);
        if let Some(run) = run {
            window.dispatch_action(run, cx);
        }
    }

    /// Every shortcut of the current list, above the footer's "all keys".
    /// Clicking one closes the dropdown and runs it.
    fn render_shortcuts(
        &self,
        shortcuts: Vec<Shortcut>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = self.theme;
        let rows: Vec<_> = shortcuts
            .into_iter()
            .enumerate()
            .map(|(ix, shortcut)| {
                div()
                    .id(("shortcut", ix))
                    .mx_1()
                    .px_2()
                    .h(px(28.))
                    .flex_none()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(160.))
                            .flex_none()
                            .text_color(rgb(t.text))
                            .child(shortcut.keys.join("  ")),
                    )
                    .child(div().text_color(rgb(t.muted)).child(shortcut.action))
                    .when(ix == self.shortcut_selected, |d| d.bg(rgb(t.selected)))
                    .when_some(shortcut.run, |row, run| {
                        row.cursor_pointer()
                            .hover(|d| d.bg(rgb(t.selected)))
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
            .bottom(px(FOOTER_HEIGHT + 4.))
            .w(px(440.))
            .max_h(px(400.))
            .overflow_y_scroll()
            .track_scroll(&self.shortcuts_scroll)
            .occlude()
            .py_1()
            .flex()
            .flex_col()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_md()
            .shadow_lg()
            .text_size(px(SMALL_FONT_SIZE))
            .children(rows)
    }
}
