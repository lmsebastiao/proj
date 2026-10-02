//! Drawing the palette.

use std::{cmp::Ordering, ops::Range, path::PathBuf};

use gpui::{
    AnyElement, Context, Div, Focusable, FontWeight, HighlightStyle, KeyDownEvent,
    ModifiersChangedEvent, MouseButton, StyledText, Window, div, list, prelude::*, px, rgb, rgba,
};

use crate::{input, paths, store, templates::Template};

use super::{
    Palette,
    actions::{ActionEntry, MenuKind},
    app_icon::{self, app_icon},
    items::*,
    keymap::*,
    scrollbar::LIST_PADDING,
    secondary,
    shortcuts::Shortcut,
    switch::DraggedWindow,
    theme::*,
    tooltip::tooltip,
};

/// The group of a list row, so its icons can show while the mouse is over it.
const ROW_GROUP: &str = "row";

/// A key as key caps: "ctrl-k" as [ctrl] [k], "↑ ↓" as [↑] [↓]. Words that
/// aren't modifiers ("right-click", "#tag") stay in one cap.
pub(super) fn keycaps(keys: &str, t: Theme) -> Div {
    const MODIFIERS: [&str; 4] = ["ctrl", "alt", "shift", "cmd"];
    let caps: Vec<String> = keys
        .split_whitespace()
        .flat_map(|key| {
            let parts: Vec<&str> = key.split('-').collect();
            let chord = parts.len() > 1
                && parts.iter().all(|p| !p.is_empty())
                && parts[..parts.len() - 1]
                    .iter()
                    .all(|p| MODIFIERS.contains(p));
            if chord {
                parts.into_iter().map(str::to_string).collect()
            } else {
                vec![key.to_string()]
            }
        })
        .collect();
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(3.))
        .children(caps.into_iter().map(move |cap| {
            div()
                .h(px(20.))
                .min_w(px(20.))
                .px(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(rgb(t.border))
                .bg(rgb(t.hover))
                .text_size(px(12.))
                .text_color(rgb(t.muted))
                .child(cap)
        }))
}

/// A glyph of the icon font, centred in the row's icon slot.
fn glyph(icon: &'static str, color: u32) -> Div {
    div()
        .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
        .text_size(px(16.))
        .text_color(rgb(color))
        .child(icon)
}

/// A program's icon, or `fallback` where the platform has none to show
/// (Linux), so the slot isn't left empty.
fn program_icon(program: Option<PathBuf>, fallback: &'static str, t: Theme) -> AnyElement {
    match program {
        Some(program) if crate::platform::HAS_APP_ICONS => {
            app_icon(Some(program), app_icon::ROW_SIZE).into_any_element()
        }
        _ if crate::platform::HAS_APP_ICONS => {
            app_icon(None, app_icon::ROW_SIZE).into_any_element()
        }
        _ => glyph(fallback, t.muted).into_any_element(),
    }
}

/// A check box, numbered with its place in the order when ticked (`position`).
fn check_box(position: Option<usize>, t: Theme) -> AnyElement {
    div()
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
        .into_any_element()
}

/// A group's icon: folders, with how many on a badge.
fn group_icon(count: usize, t: Theme) -> AnyElement {
    div()
        .relative()
        .size(px(ICON_SLOT))
        .flex()
        .items_center()
        .justify_center()
        .child(glyph(icons::GROUP, t.muted))
        .child(
            div()
                .absolute()
                .right(px(-3.))
                .bottom(px(-2.))
                .min_w(px(13.))
                .h(px(13.))
                .px(px(2.))
                .rounded_full()
                .bg(rgb(t.accent))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.))
                .text_color(rgb(t.bg))
                .child(count.to_string()),
        )
        .into_any_element()
}

/// The accent bar at the start of the highlighted row.
fn selection_bar(t: Theme, height: f32) -> Div {
    div()
        .absolute()
        .left(px(1.))
        .top(px((height - 18.) / 2.))
        .w(px(3.))
        .h(px(18.))
        .rounded_full()
        .bg(rgb(t.accent))
}

impl PaletteCommand {
    fn icon(self) -> &'static str {
        match self {
            Self::Autostart => icons::POWER,
            Self::AddProjects => icons::ADD,
            Self::NewFromTemplate => icons::NEW_PROJECT,
            Self::ChangeEditor => icons::EDIT,
            Self::Theme => icons::THEME,
            Self::OpenConfig => icons::SETTINGS,
            Self::RemoveMissing => icons::REMOVE,
            Self::Update => icons::UPDATE,
            Self::Quit => icons::QUIT,
        }
    }
}

impl Palette {
    /// What goes at the start of a row, the same width in every list: the
    /// editor's or window's program icon, or a glyph; while marking, a
    /// project's check box; while ctrl is held, its number.
    fn row_icon(&self, row: usize, ix: usize) -> AnyElement {
        let t = self.theme;
        let slot = div()
            .flex_none()
            .size(px(ICON_SLOT))
            .flex()
            .items_center()
            .justify_center();
        let content: AnyElement = match self.list() {
            List::Group => {
                let path = &self.projects[ix].path;
                let position = self
                    .group_page
                    .as_ref()
                    .and_then(|page| page.ticked.iter().position(|p| p == path));
                check_box(position, t)
            }
            List::Projects => {
                let project = &self.projects[ix];
                if !self.marked.is_empty() && !project.is_workspace() {
                    // Numbered in the order marked, the folder order in the group.
                    let position = self.marked.iter().position(|p| p == &project.path);
                    check_box(position, t)
                } else if self.numbers_shown && row < 9 {
                    keycaps(&(row + 1).to_string(), t).into_any_element()
                } else if project.missing {
                    glyph(icons::MISSING, t.danger).into_any_element()
                } else if project.is_workspace() {
                    group_icon(project.paths().len(), t)
                } else {
                    let command = project
                        .editor
                        .as_ref()
                        .or(self.config.editor.as_ref())
                        .filter(|c| !c.trim().is_empty());
                    match command.and_then(|c| self.apps.get(c).cloned().flatten()) {
                        Some(program) => program_icon(Some(program), icons::FOLDER, t),
                        None => glyph(icons::FOLDER, t.muted).into_any_element(),
                    }
                }
            }
            List::Browse => {
                let is_dir = self.browse.as_ref().is_some_and(|b| b.entries[ix].is_dir);
                glyph(if is_dir { icons::FOLDER } else { icons::FILE }, t.muted).into_any_element()
            }
            List::Editors | List::OpenWith => match &self.editors[ix] {
                EditorOption::Detected(editor) => {
                    program_icon(editor.app.clone(), icons::PROGRAM, t)
                }
                EditorOption::Browse => glyph(icons::PROGRAM, t.muted).into_any_element(),
                EditorOption::FileManager => glyph(icons::FOLDER, t.muted).into_any_element(),
            },
            List::Switch => program_icon(
                Some(self.windows[self.switch_rows[ix][0]].exe.clone()),
                icons::PROGRAM,
                t,
            ),
            List::Commands => glyph(self.commands[ix].icon(), t.muted).into_any_element(),
            List::Templates => match self.templates[ix] {
                Template::Folder(_) => glyph(icons::FOLDER, t.muted).into_any_element(),
                Template::Git(_) => glyph(icons::GLOBE, t.muted).into_any_element(),
            },
            List::Forges => glyph(icons::GLOBE, t.muted).into_any_element(),
            List::Text => div().into_any_element(),
        };
        slot.child(content).into_any_element()
    }

    pub(super) fn render_row(
        &self,
        row: usize,
        now: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = self.theme;
        let m = &self.matches[row];
        let selected = row == self.selected;
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
        let is_projects = self.list() == List::Projects;
        // Projects with an editor window open (the switcher lists those).
        let open = is_projects && self.open_keys.contains(&self.projects[m.ix].key());
        // A switcher row's number, for the number keys while holding the
        // switcher and the switch-by-number shortcuts (alt+shift+1…9). The ones
        // past 9 keep the space so the titles line up. (The project list's
        // ctrl+1…9 numbers take the icon's place instead, so nothing moves.)
        let number = (self.list() == List::Switch
            && (self.hold.is_some() || self.config.switch_number_modifiers().is_some()))
        .then(|| {
            div()
                .flex_none()
                .w(px(12.))
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.muted))
                .when(m.ix < 9, |d| d.child((m.ix + 1).to_string()))
        });
        // Switcher rows can be dragged to another place in the list (it shows
        // every row in order then, so `row` is also the row's index).
        let dragged = self.can_reorder().then(|| DraggedWindow {
            ix: m.ix,
            title: title.clone().into(),
            width: px(0.),
            theme: t,
        });
        let icon = self.row_icon(row, m.ix);
        // A project whose folder is gone is dimmed; tags show after the name.
        let project = is_projects.then(|| &self.projects[m.ix]);
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
                .when(!selected, |d| {
                    d.invisible().group_hover(ROW_GROUP, |s| s.visible())
                })
                .child(icons::CLOSE)
        });
        // The actions menu: shown on the highlighted row and on any row the
        // mouse is over.
        let more = is_projects.then(|| {
            div()
                .id(("more", row))
                .flex_none()
                .tooltip(tooltip(
                    format!("All actions · {}-k or right-click", secondary()),
                    t,
                ))
                .size(px(28.))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                .text_size(px(14.))
                .text_color(rgb(t.muted))
                .hover(|d| d.bg(rgb(t.border)).text_color(rgb(t.text)))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    // Not also a click on the row, which opens the project.
                    cx.stop_propagation();
                    this.actions_for_row(row, cx);
                }))
                .when(!selected, |d| {
                    d.invisible().group_hover(ROW_GROUP, |s| s.visible())
                })
                .child(icons::MORE)
        });

        // A group's folders by name, each with a dot when its own window is open.
        let members_line = project.filter(|p| p.is_workspace()).map(|group| {
            let mut parts: Vec<AnyElement> = Vec::new();
            for (i, (path, listed)) in self.members(group).into_iter().enumerate() {
                if i > 0 {
                    parts.push(div().flex_none().child("·").into_any_element());
                }
                parts.push(
                    div()
                        .flex_none()
                        .child(member_name(&path, listed))
                        .into_any_element(),
                );
                if listed.is_some_and(|m| self.open_keys.contains(&m.key())) {
                    parts.push(
                        div()
                            .flex_none()
                            .size(px(6.))
                            .rounded_full()
                            .bg(rgb(t.open))
                            .into_any_element(),
                    );
                }
            }
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.muted))
                .children(parts)
        });
        // A line between the projects with a window open and the rest, while
        // the list shows them all in order, with a little more space.
        let starts_section = self.starts_section(row);

        // The list lays each row out on its own, so both levels need an explicit width.
        // Its height is `row_height`'s, which the scrollbar goes by.
        div()
            .w_full()
            .px_2()
            .pt(px(
                ROW_GAP / 2. + if starts_section { SECTION_GAP } else { 0. }
            ))
            .pb(px(ROW_GAP / 2.))
            .relative()
            .when(starts_section, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(SECTION_GAP / 2.))
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
                    .relative()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .px_3()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_3()
                    .map(|d| {
                        if selected {
                            d.bg(rgb(t.selected)).child(selection_bar(t, ROW_HEIGHT))
                        } else {
                            d.hover(|d| d.bg(rgb(t.hover)))
                        }
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected = row;
                        this.confirm(&Confirm, window, cx);
                    }))
                    // Right-click: the project's actions, as a context menu would.
                    .when(is_projects, |d| {
                        d.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _, _, cx| {
                                this.actions_for_row(row, cx);
                            }),
                        )
                    })
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
                    .child(icon)
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
                            .child(match members_line {
                                Some(line) => line,
                                None => div()
                                    .text_size(px(SMALL_FONT_SIZE))
                                    .text_color(rgb(t.muted))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(StyledText::new(subtitle).with_highlights(subtitle_hl)),
                            }),
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
                                .id(("meta", row))
                                .flex_none()
                                .max_w(px(280.))
                                .flex()
                                .flex_col()
                                .items_end()
                                .text_size(px(SMALL_FONT_SIZE))
                                .children(meta.top.map(|(text, color)| line(text, color)))
                                .children(meta.bottom.map(|text| line(text, t.muted)))
                                // What the branch's marks, "missing" or "· →" mean.
                                .when_some(meta.tip, |d, tip| d.tooltip(tooltip(tip, t))),
                        )
                    })
                    .children(more)
                    .children(close),
            )
    }

    /// The actions menu (ctrl-k), over the list at the bottom right like
    /// PowerToys' Command Palette: its own search box, then one-line rows
    /// with an icon under section headings. A plain list rather than a
    /// `uniform_list`, as the headings make rows differ in height.
    fn render_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let highlight = HighlightStyle {
            color: Some(rgb(t.accent).into()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let mut entries: Vec<AnyElement> = self
            .action_entries()
            .into_iter()
            .map(|entry| match entry {
                ActionEntry::Heading(label) => div()
                    .px_4()
                    .pt_2()
                    .pb_1()
                    .text_size(px(11.))
                    .text_color(rgb(t.muted))
                    .child(label.to_uppercase())
                    .into_any_element(),
                ActionEntry::Line => div()
                    .mx_3()
                    .my_1()
                    .h(px(1.))
                    .bg(rgb(t.border))
                    .into_any_element(),
                ActionEntry::Row(row) => {
                    let m = &self.menu_matches[row];
                    let action = self.actions[m.ix];
                    let (title, subtitle) = self.action_text(action);
                    let detail = self.action_detail(action);
                    let selected = row == self.menu_selected;
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
                        .relative()
                        .mx_1()
                        .px_3()
                        .h(px(32.))
                        .flex_none()
                        .rounded_md()
                        .flex()
                        .items_center()
                        .gap_3()
                        .map(|d| {
                            if selected {
                                d.bg(rgb(t.selected)).child(selection_bar(t, 32.))
                            } else {
                                d.hover(|d| d.bg(rgb(t.hover)))
                            }
                        })
                        // What it does, when the mouse rests on it.
                        .when(!subtitle.is_empty(), |d| d.tooltip(tooltip(subtitle, t)))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.menu_selected = row;
                            this.confirm_menu(window, cx);
                        }))
                        .child(
                            div()
                                .flex_none()
                                .w(px(18.))
                                .flex()
                                .justify_center()
                                .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                                .text_size(px(14.))
                                .text_color(rgb(icon_color))
                                .child(action.icon()),
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
                        // Its key, or the project's tags.
                        .children(detail.map(|detail| {
                            if detail.starts_with('#') {
                                div()
                                    .flex_none()
                                    .max_w(px(160.))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(px(SMALL_FONT_SIZE))
                                    .text_color(rgb(t.muted))
                                    .child(detail)
                            } else {
                                keycaps(&detail, t)
                            }
                        }))
                        .into_any_element()
                }
            })
            .collect();
        if self.menu_matches.is_empty() {
            entries.push(
                div()
                    .px_4()
                    .py_2()
                    .text_size(px(SMALL_FONT_SIZE))
                    .text_color(rgb(t.muted))
                    .child(match self.menu_kind {
                        MenuKind::Actions => "No matching actions",
                        MenuKind::Commands => "No matching commands",
                    })
                    .into_any_element(),
            );
        }
        div()
            .id("menu")
            .absolute()
            .right(px(8.))
            .bottom(px(FOOTER_HEIGHT + 4.))
            .w(px(420.))
            .max_h(px(380.))
            .occlude()
            .flex()
            .flex_col()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_lg()
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .h(px(42.))
                    .px_4()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(t.border))
                    .text_size(px(FONT_SIZE))
                    .child(self.menu_input.clone()),
            )
            .child(
                div()
                    .id("actions")
                    .flex_shrink()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.actions_scroll)
                    .py_1()
                    .flex()
                    .flex_col()
                    .children(entries),
            )
    }

    pub(super) fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        if self.add_candidate.is_some()
            || self.clone_candidate.is_some()
            || self.list() == List::Text
        {
            return div().flex_1().into_any_element();
        }
        let centered = || {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .px_6()
                .text_size(px(FONT_SIZE))
                .text_color(rgb(t.muted))
        };
        let add_button = |cx: &mut Context<Self>| {
            div()
                .id("add-projects")
                .px_3()
                .h(px(32.))
                .rounded_md()
                .flex()
                .items_center()
                .gap_2()
                .bg(rgb(t.selected))
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.text))
                .cursor_pointer()
                .hover(|d| d.bg(rgb(t.border)))
                .on_click(
                    cx.listener(|this, _, window, cx| this.add_projects(&AddProjects, window, cx)),
                )
                .child("Add projects…")
                .child(keycaps(&format!("{}-o", secondary()), t))
        };
        if self.list() == List::Templates && self.templates.is_empty() {
            return centered()
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
            return centered()
                .child("No projects yet")
                .child(add_button(cx))
                .child(
                    div()
                        .text_size(px(SMALL_FONT_SIZE))
                        .child("or paste a folder path above"),
                )
                .into_any_element();
        }
        // Not there: say how to get it there.
        if self.list() == List::Projects {
            return centered()
                .child(format!("No projects match \"{}\"", self.filter_query()))
                .child(add_button(cx))
                .child(
                    div()
                        .text_size(px(SMALL_FONT_SIZE))
                        .child("or paste a folder path or a git URL to add or clone it"),
                )
                .into_any_element();
        }
        let empty = match self.list() {
            List::Switch if self.windows.is_empty() => {
                crate::platform::window_access_hint().unwrap_or("No editor windows are open")
            }
            List::Commands => "No commands match",
            _ => "No matches",
        };
        div()
            .flex_1()
            .p_4()
            .text_size(px(FONT_SIZE))
            .text_color(rgb(t.muted))
            .child(empty)
            .into_any_element()
    }

    /// A line under the search bar on pages that need saying what enter does.
    /// The page's name is in the search bar; the keys are in the footer.
    pub(super) fn render_banner(&self) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let m = secondary();
        let (title, lines): (Option<String>, Vec<String>) = match self.mode {
            Mode::Projects | Mode::Switch | Mode::Browse | Mode::Templates => return None,
            Mode::Group if self.group_page.as_ref()?.editing.is_some() => (
                None,
                vec!["Tick or untick folders (tab or space); alt-↑/↓ moves the highlighted one in the order they open in. ↵ saves.".into()],
            ),
            Mode::Group => (
                None,
                vec![format!(
                    "Tick projects (tab or space) to open in one editor window, in that order. ↵ opens them once; {}-↵ saves them as a group.",
                    secondary()
                )],
            ),
            Mode::Forges => {
                let pick = self.forge_pick.as_ref()?;
                let line = match pick.opens() {
                    Some(opens) => format!(
                        "proj can't tell from its name. ↵ opens its {opens} and saves the choice \
                         under forges in the config file, so it won't ask again."
                    ),
                    None => "For its pull request and CI links. ↵ saves the choice under forges \
                             in the config file."
                        .into(),
                };
                (None, vec![line])
            }
            Mode::Tags => (
                None,
                vec!["Words, separated by spaces or commas; then search for #tag to list just those projects.".into()],
            ),
            Mode::AddCommand => {
                let project = self.edited_project()?;
                (
                    None,
                    vec![format!(
                        "It runs in a terminal in {}, from the project's commands ({m}-r).",
                        project.location()
                    )],
                )
            }
            Mode::NewProject => {
                let into = match self.config.scan_dirs.first() {
                    Some(dir) => format!("in {}", paths::display_path(dir)),
                    None => "in a folder you pick next".into(),
                };
                (
                    None,
                    vec![format!(
                        "Made {into}, with a git history of its own, then opened."
                    )],
                )
            }
            // First run: nothing to go back to, so it says what proj is about.
            Mode::Editors if self.config.editor.is_none() => (
                Some("Which editor should open your projects?".into()),
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
                None,
                vec![format!(
                    "For every project without its own editor. Just one project? {m}-↵ on it instead."
                )],
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
                    None,
                    vec![format!(
                        "Search still finds it by {}. Empty goes back to the folder name.",
                        folders.join(" and ")
                    )],
                )
            }
            Mode::OpenWith => {
                let project = self.open_with_project()?;
                let name = &project.name;
                let editor = self.name_of(&project.default_editor(&self.config));
                let now = match project.editor {
                    Some(_) => format!("Its own default is {editor}."),
                    None => format!("It opens in the default editor, {editor}."),
                };
                (
                    None,
                    vec![format!(
                        "{now} ↵ opens {name} with another just this once; {m}-↵ makes that its default."
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
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.muted))
                .children(title.map(|title| {
                    div()
                        .text_size(px(FONT_SIZE))
                        .text_color(rgb(t.text))
                        .child(title)
                }))
                .children(lines),
        )
    }

    /// The search bar: on a page off the project list, a back button and the
    /// page's name before the search box; after `>` or `@`, what's searched.
    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let page = self.page_title();
        let top_level = page.is_none();
        let scope = match self.list() {
            List::Commands => Some("Commands"),
            List::Switch if self.mode == Mode::Projects && self.expanded.is_none() => {
                Some("Windows")
            }
            _ => None,
        };
        div()
            .flex_none()
            .h(px(SEARCH_HEIGHT))
            .px_3()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(t.border))
            .when_some(page, |d, page| {
                d.child(
                    div()
                        .id("back")
                        .flex_none()
                        .size(px(30.))
                        .rounded_md()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                        .text_size(px(14.))
                        .text_color(rgb(t.muted))
                        .hover(|d| d.bg(rgb(t.hover)).text_color(rgb(t.text)))
                        .cursor_pointer()
                        .tooltip(tooltip("Back · esc, or backspace with nothing typed", t))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.go_back(cx);
                        }))
                        .child(icons::BACK),
                )
                .child(
                    div()
                        .flex_none()
                        .max_w(px(280.))
                        .h(px(26.))
                        .px_2()
                        .rounded_md()
                        .bg(rgb(t.selected))
                        .flex()
                        .items_center()
                        .text_size(px(SMALL_FONT_SIZE))
                        .text_color(rgb(t.text))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(page),
                        ),
                )
            })
            .when(top_level, |d| d.pl_4())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(INPUT_FONT_SIZE))
                    .line_height(px(27.))
                    .child(self.input.clone()),
            )
            .children(scope.map(|scope| {
                div()
                    .flex_none()
                    .pr_2()
                    .text_size(px(SMALL_FONT_SIZE))
                    .text_color(rgb(t.muted))
                    .child(scope)
            }))
    }

    /// What the footer says on the left when there's no message.
    fn footer_context(&self) -> Option<String> {
        Some(match self.list() {
            List::Group => return self.ticked_names(),
            List::Projects if !self.marked.is_empty() => {
                let names: Vec<String> = self
                    .marked
                    .iter()
                    .filter_map(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .collect();
                format!("{} · esc clears", names.join(" + "))
            }
            List::Projects if !self.filter_query().is_empty() => {
                format!("{} of {} projects", self.matches.len(), self.projects.len())
            }
            List::Projects => format!("{} projects", self.projects.len()),
            List::Browse => format!("{} items", self.item_count()),
            List::Switch if self.hold.is_some() => "Let go to switch".into(),
            List::Switch => format!("{} windows", self.windows.len()),
            _ => return None,
        })
    }

    /// Something went wrong: in full, wrapped, in the warning colour, above
    /// the footer.
    fn render_problem(&self) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let status = self.status.as_ref().filter(|s| s.problem)?;
        Some(
            div()
                .flex_none()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(rgb(t.border))
                .flex()
                .items_start()
                .gap_2()
                .text_size(px(SMALL_FONT_SIZE))
                .text_color(rgb(t.danger))
                .child(
                    div()
                        .flex_none()
                        .pt(px(2.))
                        .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                        .text_size(px(12.))
                        .child(icons::WARNING),
                )
                .child(div().flex_1().min_w_0().child(status.text.clone())),
        )
    }

    /// Left: a message, or what the list holds. Right: buttons for the list's
    /// main two keys, as in PowerToys' Command Palette, after a ? for all of them.
    fn render_footer(
        &self,
        shortcuts: &[Shortcut],
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let t = self.theme;
        let info = self.status.as_ref().filter(|s| !s.problem);
        let left = match info {
            Some(status) => div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .text_color(rgb(t.accent))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(status.text.clone()),
                )
                .when(status.undo, |d| {
                    d.child(
                        div()
                            .id("undo")
                            .flex_none()
                            .px_2()
                            .h(px(24.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_color(rgb(t.text))
                            .hover(|d| d.bg(rgb(t.hover)))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.undo_last_remove(cx)))
                            .child("Undo")
                            .child(keycaps(&format!("{}-z", secondary()), t)),
                    )
                }),
            None => div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .children(self.footer_context()),
        };
        let help = shortcuts.iter().any(|s| s.footer.is_none()).then(|| {
            div()
                .id("all-keys")
                .flex_none()
                .size(px(28.))
                .rounded_md()
                .flex()
                .items_center()
                .justify_center()
                .when(!icons::FONT.is_empty(), |d| d.font_family(icons::FONT))
                .text_size(px(13.))
                .text_color(rgb(if self.show_shortcuts {
                    t.accent
                } else {
                    t.muted
                }))
                .hover(|d| d.bg(rgb(t.hover)).text_color(rgb(t.text)))
                .cursor_pointer()
                .tooltip(tooltip("Every key of this list · f1", t))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_shortcuts(cx)))
                .child(icons::HELP)
        });
        let mut buttons: Vec<AnyElement> = Vec::new();
        for (ix, shortcut) in shortcuts.iter().enumerate() {
            let Some(label) = shortcut.footer else {
                continue;
            };
            if !buttons.is_empty() {
                buttons.push(
                    div()
                        .flex_none()
                        .w(px(1.))
                        .h(px(16.))
                        .bg(rgb(t.border))
                        .into_any_element(),
                );
            }
            let run = shortcut.run.as_ref().map(|run| run.boxed_clone());
            buttons.push(
                div()
                    .id(("footer", ix))
                    .flex_none()
                    .h(px(30.))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_color(rgb(t.text))
                    .tooltip(tooltip(shortcut.action, t))
                    .when_some(run, |d, run| {
                        d.cursor_pointer()
                            .hover(|d| d.bg(rgb(t.hover)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_shortcuts = false;
                                window.dispatch_action(run.boxed_clone(), cx);
                            }))
                    })
                    .child(label)
                    .child(keycaps(&shortcut.keys[0], t))
                    .into_any_element(),
            );
        }
        div()
            .flex_none()
            .h(px(FOOTER_HEIGHT))
            .pl_4()
            .pr_1()
            .border_t_1()
            .border_color(rgb(t.border))
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(SMALL_FONT_SIZE))
            .text_color(rgb(t.muted))
            .child(left)
            .child(div().flex_1())
            .children(help)
            .children(buttons)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        // Keys go to the menu's search box while it's open, else the main one.
        let focus = if self.menu_open() {
            self.menu_input.focus_handle(cx)
        } else {
            self.input.focus_handle(cx)
        };
        if !focus.is_focused(window) && window.is_window_active() {
            window.focus(&focus);
        }
        self.sync_list();
        let rows = if !self.matches.is_empty() {
            let now = store::now();
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(
                    list(
                        self.list_state.clone(),
                        cx.processor(move |this, row: usize, _, cx| {
                            this.render_row(row, now, cx).into_any_element()
                        }),
                    )
                    .size_full()
                    .py(px(LIST_PADDING)),
                )
                .children(self.render_scrollbar(cx))
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
                .relative()
                .mx_2()
                .mt_1()
                .px_3()
                .h(px(ROW_HEIGHT))
                .rounded_md()
                .bg(rgb(t.selected))
                .child(selection_bar(t, ROW_HEIGHT))
                .flex()
                .items_center()
                .gap_3()
                .text_size(px(FONT_SIZE))
                .child(
                    div()
                        .flex_none()
                        .size(px(ICON_SLOT))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(glyph(icons::ADD, t.accent)),
                )
                .child(div().flex_none().text_color(rgb(t.accent)).child(label))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_color(rgb(t.text))
                        .child(detail),
                )
                .child(div().flex_1())
                .child(keycaps("↵", t))
        });

        let shortcuts = self.shortcuts();
        let footer = self.render_footer(&shortcuts, cx);
        let dropdown = self
            .show_shortcuts
            .then(|| self.render_shortcuts(shortcuts, cx));
        // The menu, over a backdrop that closes it when clicked, as clicking
        // outside a context menu does.
        let menu = self.menu_open().then(|| {
            (
                // Not over the footer, whose buttons work on the menu.
                div()
                    .id("backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom(px(FOOTER_HEIGHT))
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| this.close_menu(cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| this.close_menu(cx)),
                    ),
                self.render_menu(cx),
            )
        });
        let (backdrop, menu) = match menu {
            Some((backdrop, menu)) => (Some(backdrop), Some(menu)),
            None => (None, None),
        };

        div()
            .key_context("Palette")
            // While the switcher is held open, a number switches to that window
            // and other keys start a search.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.menu_open() {
                    return;
                }
                // Space ticks on the group page (no spaces in project names to type).
                if this.list() == List::Group
                    && event.keystroke.key == "space"
                    && !event.keystroke.modifiers.modified()
                {
                    this.toggle_tick(0, cx);
                    cx.stop_propagation();
                    return;
                }
                if this.switch_to_number(&event.keystroke.key, window, cx)
                    || this.open_number(&event.keystroke, window, cx)
                    || this.type_in_switcher(&event.keystroke, window, cx)
                {
                    cx.stop_propagation();
                }
            }))
            // Ctrl (cmd) or alt alone held: the projects show the numbers
            // ctrl+1…9 and alt+1…9 open.
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                let m = event.modifiers;
                let ctrl = m.secondary() && !m.alt;
                let alt = ALT_ACTIONS && m.alt && !m.secondary();
                this.ctrl_held((ctrl || alt) && !m.shift, cx);
            }))
            // Home/end move the text cursor; with nothing typed, the selection.
            .capture_action(cx.listener(|this, _: &input::Home, _, cx| {
                if this.menu_open() {
                    if this.menu_query.is_empty() {
                        this.select_menu_row(0, cx);
                        cx.stop_propagation();
                    }
                } else if this.query.is_empty() && !this.matches.is_empty() {
                    this.select_row(0, cx);
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::End, _, cx| {
                if this.menu_open() {
                    if this.menu_query.is_empty() {
                        this.select_menu_row(usize::MAX, cx);
                        cx.stop_propagation();
                    }
                } else if this.query.is_empty() && !this.matches.is_empty() {
                    this.select_row(usize::MAX, cx);
                    cx.stop_propagation();
                }
            }))
            // Backspace with nothing typed goes back a page, as in PowerToys'
            // Command Palette; in the menu, closes it. Not while typing a name
            // or tags, where empty is a value.
            .capture_action(cx.listener(|this, _: &input::Backspace, _, cx| {
                if this.menu_open() {
                    if this.menu_input.read(cx).text().is_empty() {
                        this.close_menu(cx);
                        cx.stop_propagation();
                    }
                    return;
                }
                if this.list() != List::Text
                    && this.input.read(cx).text().is_empty()
                    && this.go_back(cx)
                {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPageDown, _, cx| this.select_page(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPageUp, _, cx| this.select_page(-1, cx)))
            .on_action(cx.listener(Self::undo_remove))
            // →/← browse into projects and folders, but only at the ends of the
            // search text, so they still move the cursor while editing it.
            .capture_action(cx.listener(|this, _: &input::Right, _, cx| {
                if !this.menu_open() && this.input.read(cx).cursor_at_end() && this.enter(cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &input::Left, _, cx| {
                if this.menu_open() {
                    return;
                }
                // Out of a project's windows also from just after the `@` that
                // lists them in the project search.
                let at_start = this.input.read(cx).cursor_at_start()
                    || (this.list() == List::Switch && this.filter_query().is_empty());
                if at_start && this.leave(cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &RemoveItem, _, cx| {
                if this.menu_open() {
                    this.remove_task(cx);
                } else if this.list() == List::Projects {
                    this.remove(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameItem, _, cx| {
                if this.list() == List::Projects {
                    this.close_menu(cx);
                    this.rename(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenConfig, window, cx| {
                this.close_menu(cx);
                this.run_command(PaletteCommand::OpenConfig, window, cx);
            }))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.select(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.select(-1, cx)))
            .on_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.list() == List::Group {
                    this.move_tick(1, cx);
                } else {
                    this.select(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.list() == List::Group {
                    this.move_tick(-1, cx);
                } else {
                    this.select(-1, cx);
                }
            }))
            .on_action(cx.listener(Self::confirm))
            // The row's second action: in the Open-with list, make the editor
            // the project's default; on a project, open with…
            .on_action(
                cx.listener(|this, _: &ConfirmSecondary, window, cx| match this.list() {
                    List::OpenWith => this.toggle_project_default(window, cx),
                    List::Group => this.save_ticked(cx),
                    List::Projects if !this.menu_open() => this.open_with_selected(cx),
                    _ => {}
                }),
            )
            .on_action(cx.listener(|this, _: &ShowInFileManager, window, cx| {
                this.close_menu(cx);
                this.open_selected(Target::FileManager, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenTerminal, window, cx| {
                this.close_menu(cx);
                this.open_selected(Target::Terminal, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenWithMenu, _, cx| {
                this.close_menu(cx);
                this.open_with_selected(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMark, _, cx| {
                if this.menu_open() {
                    this.select_in_menu(1, cx);
                } else {
                    this.toggle_mark(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleMarkUp, _, cx| {
                if this.menu_open() {
                    this.select_in_menu(-1, cx);
                } else {
                    this.toggle_mark(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowActions, _, cx| this.show_actions(cx)))
            .on_action(cx.listener(|this, _: &ShowCommands, _, cx| this.show_commands(cx)))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(|this, action: &OpenRemote, window, cx| {
                this.close_menu(cx);
                this.open_remote(action, window, cx);
            }))
            .on_action(cx.listener(|this, action: &AddProjects, window, cx| {
                this.close_menu(cx);
                this.add_projects(action, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleShortcuts, _, cx| {
                this.close_menu(cx);
                this.toggle_shortcuts(cx);
            }))
            .on_action(cx.listener(Self::dismiss))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_lg()
            .overflow_hidden()
            .text_color(rgb(t.text))
            .child(self.render_search_bar(cx))
            .children(self.render_banner())
            .children(add_row)
            .child(rows)
            .children(self.render_problem())
            .child(footer)
            .children(backdrop)
            .children(menu)
            .children(dropdown)
    }
}

impl Palette {
    /// Alt-enter or ctrl-enter on a project, or the marked ones: the Open-with list.
    fn open_with_selected(&mut self, cx: &mut Context<Self>) {
        let entry = if self.marked.len() > 1 {
            self.marked_group()
        } else {
            self.selected_project().cloned()
        };
        if let Some(entry) = entry {
            self.show_open_with(entry.key(), cx);
        }
    }

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

    /// Every shortcut of the current list, above the footer's ? button.
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
                let selected = ix == self.shortcut_selected;
                div()
                    .id(("shortcut", ix))
                    .relative()
                    .mx_1()
                    .px_2()
                    .h(px(30.))
                    .flex_none()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .w(px(190.))
                            .flex_none()
                            .flex()
                            .gap_2()
                            .children(shortcut.keys.iter().map(|key| keycaps(key, t))),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_color(rgb(if shortcut.run.is_some() {
                                t.text
                            } else {
                                t.muted
                            }))
                            .child(shortcut.action),
                    )
                    .when(selected, |d| {
                        d.bg(rgb(t.selected)).child(selection_bar(t, 30.))
                    })
                    .when_some(shortcut.run, |row, run| {
                        row.cursor_pointer()
                            .when(!selected, |d| d.hover(|d| d.bg(rgb(t.hover))))
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
            .w(px(520.))
            .max_h(px(380.))
            .overflow_y_scroll()
            .track_scroll(&self.shortcuts_scroll)
            .occlude()
            .py_1()
            .flex()
            .flex_col()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .rounded_lg()
            .shadow_lg()
            .text_size(px(SMALL_FONT_SIZE))
            .children(rows)
    }
}
