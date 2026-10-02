//! Where the list's rows sit, and the scrollbars of the list and the menu.

use gpui::{
    Context, DragMoveEvent, Empty, ListOffset, MouseButton, MouseDownEvent, Pixels, Render, Window,
    div, point, prelude::*, px, rgb,
};

use super::{Palette, items::List, theme::*};

/// The list's padding above its first row and below its last.
pub(super) const LIST_PADDING: f32 = 2.;
/// The space a scrollbar takes at the right edge.
pub(super) const SCROLLBAR_WIDTH: f32 = THUMB_WIDTH + 2.;
const THUMB_WIDTH: f32 = 6.;
const MIN_THUMB_HEIGHT: f32 = 24.;
/// The space between a scrollbar and the top and bottom of what it scrolls.
const INSET: f32 = 4.;

/// What a scrollbar scrolls.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Scrolled {
    /// The main list.
    List,
    /// The actions menu (ctrl-k).
    Menu,
}

impl Scrolled {
    fn name(self) -> &'static str {
        match self {
            Scrolled::List => "list-scrollbar",
            Scrolled::Menu => "menu-scrollbar",
        }
    }
}

/// A drag of a scrollbar. Where the thumb was taken hold of is on the palette.
pub(super) struct ScrollbarDrag(Scrolled);

impl Render for ScrollbarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// How much there is to scroll, how much shows, and how far it's scrolled.
#[derive(Clone, Copy, PartialEq)]
struct Extent {
    viewport: f32,
    content: f32,
    scroll: f32,
}

impl Extent {
    fn max_scroll(self) -> f32 {
        (self.content - self.viewport).max(0.)
    }
}

/// A scrollbar's thumb, down its track.
struct Thumb {
    top: f32,
    height: f32,
    /// How far the thumb's top can go.
    travel: f32,
}

impl Palette {
    /// The row the projects without a window open start at, while the list
    /// shows them all in order: the open ones first.
    fn section_start(&self) -> Option<usize> {
        if self.list() != List::Projects
            || self.open_keys.is_empty()
            || !self.filter_query().is_empty()
        {
            return None;
        }
        let row = self.matches.iter().position(|m| !self.is_open(m.ix))?;
        (row > 0).then_some(row)
    }

    /// New rows: tells the list, and goes back to the top.
    pub(super) fn reset_list(&mut self) {
        self.section = self.section_start();
        self.list_state.reset(self.matches.len());
    }

    /// Tells the list when its rows' number or heights have changed, keeping
    /// its place.
    pub(super) fn sync_list(&mut self) {
        if self.list_state.item_count() != self.matches.len()
            || self.section_start() != self.section
        {
            let top = self.list_state.logical_scroll_top();
            self.reset_list();
            self.list_state.scroll_to(top);
        }
    }

    /// Whether the line between the open projects and the rest goes above `row`.
    pub(super) fn starts_section(&self, row: usize) -> bool {
        self.section == Some(row)
    }

    /// A row's height, with the space above and below it.
    pub(super) fn row_height(&self, row: usize) -> f32 {
        let section = if self.starts_section(row) {
            SECTION_GAP
        } else {
            0.
        };
        ROW_HEIGHT + ROW_GAP + section
    }

    /// How far `row` starts below the first.
    fn row_top(&self, row: usize) -> f32 {
        let section = match self.section {
            Some(start) if row > start => SECTION_GAP,
            _ => 0.,
        };
        row as f32 * (ROW_HEIGHT + ROW_GAP) + section
    }

    fn viewport_height(&self) -> f32 {
        f32::from(self.list_state.viewport_bounds().size.height)
    }

    fn content_height(&self) -> f32 {
        self.row_top(self.matches.len()) + 2. * LIST_PADDING
    }

    fn max_scroll(&self) -> f32 {
        (self.content_height() - self.viewport_height()).max(0.)
    }

    /// How far the list is scrolled.
    fn scroll_top(&self) -> f32 {
        let top = self.list_state.logical_scroll_top();
        self.row_top(top.item_ix) + f32::from(top.offset_in_item)
    }

    /// Scrolls the list `y` down, as far as its end allows.
    fn scroll_list_to(&self, y: f32) {
        let y = y.clamp(0., self.max_scroll());
        let last = self.matches.len().saturating_sub(1);
        let mut row = ((y / (ROW_HEIGHT + ROW_GAP)) as usize).min(last);
        // The line's space puts the rows after it a little lower.
        while row > 0 && self.row_top(row) > y {
            row -= 1;
        }
        self.list_state.scroll_to(ListOffset {
            item_ix: row,
            offset_in_item: px(y - self.row_top(row)),
        });
    }

    /// Scrolls `row` to the middle of the list, as far as its ends allow.
    pub(super) fn scroll_to_row(&self, row: usize) {
        let middle = LIST_PADDING + self.row_top(row) + self.row_height(row) / 2.;
        self.scroll_list_to(middle - self.viewport_height() / 2.);
    }

    fn extent(&self, of: Scrolled) -> Extent {
        match of {
            Scrolled::List => Extent {
                viewport: self.viewport_height(),
                content: self.content_height(),
                scroll: self.scroll_top(),
            },
            Scrolled::Menu => {
                let handle = &self.actions_scroll;
                let viewport = f32::from(handle.bounds().size.height);
                Extent {
                    viewport,
                    content: viewport + f32::from(handle.max_offset().height),
                    scroll: -f32::from(handle.offset().y),
                }
            }
        }
    }

    /// The top of what `of` scrolls, in the window.
    fn scrolled_top(&self, of: Scrolled) -> Pixels {
        match of {
            Scrolled::List => self.list_state.viewport_bounds().top(),
            Scrolled::Menu => self.actions_scroll.bounds().top(),
        }
    }

    fn scroll_to_y(&self, of: Scrolled, y: f32) {
        match of {
            Scrolled::List => self.scroll_list_to(y),
            Scrolled::Menu => {
                let y = y.clamp(0., self.extent(of).max_scroll());
                self.actions_scroll.set_offset(point(px(0.), px(-y)));
            }
        }
    }

    /// None while everything fits.
    fn thumb(&self, of: Scrolled) -> Option<Thumb> {
        let Extent {
            viewport,
            content,
            scroll,
        } = self.extent(of);
        if viewport <= 0. || content <= viewport {
            return None;
        }
        let track = viewport - 2. * INSET;
        let height = (track * viewport / content).clamp(MIN_THUMB_HEIGHT.min(track), track);
        let travel = track - height;
        let top = travel * (scroll / (content - viewport)).clamp(0., 1.);
        Some(Thumb {
            top,
            height,
            travel,
        })
    }

    /// Whether `of` has a scrollbar, so what it scrolls can keep clear of it.
    pub(super) fn has_scrollbar(&self, of: Scrolled) -> bool {
        self.thumb(of).is_some()
    }

    /// Puts the point of the thumb held by the mouse at `y` down the track.
    fn drag_scrollbar(&mut self, of: Scrolled, y: f32, cx: &mut Context<Self>) {
        let (Some(grab), Some(thumb)) = (self.scrollbar_grab, self.thumb(of)) else {
            return;
        };
        if thumb.travel > 0. {
            let max = self.extent(of).max_scroll();
            self.scroll_to_y(of, (y - grab) / thumb.travel * max);
            cx.notify();
        }
    }

    /// Over the right edge of what `of` scrolls, which keeps that space clear.
    pub(super) fn render_scrollbar(
        &self,
        of: Scrolled,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        // The scrollbar goes by the last layout; when the one about to be
        // done changes it (new rows, the menu just opened), draw it again.
        let drawn = self.extent(of);
        let this = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            this.update(cx, |this, cx| {
                if this.extent(of) != drawn {
                    cx.notify();
                }
            })
            .ok();
        });

        let t = self.theme;
        let thumb = self.thumb(of)?;
        let (top, height) = (thumb.top, thumb.height);
        let group = of.name();
        Some(
            div()
                .id(group)
                .group(group)
                .absolute()
                .top(px(INSET))
                .bottom(px(INSET))
                .right_0()
                .w(px(SCROLLBAR_WIDTH))
                // On the thumb: it moves with the mouse from where it was
                // taken hold of. Elsewhere: its middle jumps there first.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        let track_top = this.scrolled_top(of) + px(INSET);
                        let y = f32::from(event.position.y - track_top);
                        let on_thumb = (top..top + height).contains(&y);
                        this.scrollbar_grab = Some(if on_thumb { y - top } else { height / 2. });
                        this.drag_scrollbar(of, y, cx);
                    }),
                )
                .on_drag(ScrollbarDrag(of), move |_, _, _, cx| {
                    cx.new(|_| ScrollbarDrag(of))
                })
                // Every scrollbar hears every drag; each moves for its own.
                .on_drag_move(cx.listener(
                    move |this, event: &DragMoveEvent<ScrollbarDrag>, _, cx| {
                        if event.drag(cx).0 == of {
                            let y = f32::from(event.event.position.y - event.bounds.top());
                            this.drag_scrollbar(of, y, cx);
                        }
                    },
                ))
                .child(
                    div()
                        .absolute()
                        .top(px(top))
                        .right(px(1.))
                        .w(px(THUMB_WIDTH))
                        .h(px(height))
                        .rounded_full()
                        .bg(rgb(t.border))
                        .group_hover(group, |s| s.bg(rgb(t.muted))),
                ),
        )
    }
}
