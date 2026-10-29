//! The small box of text shown over an element the mouse rests on.

use gpui::{AnyView, App, Context, SharedString, Window, div, prelude::*, px, rgb};

use super::theme::{SMALL_FONT_SIZE, Theme};

struct Tooltip {
    text: SharedString,
    theme: Theme,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        div()
            .max_w(px(360.))
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(t.bg))
            .border_1()
            .border_color(rgb(t.border))
            .shadow_md()
            .text_size(px(SMALL_FONT_SIZE))
            .text_color(rgb(t.text))
            .child(self.text.clone())
    }
}

/// For an element's `.tooltip(…)`: `text`, in the palette's colours.
pub(super) fn tooltip(
    text: impl Into<SharedString>,
    theme: Theme,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let text = text.into();
    move |_, cx| {
        cx.new(|_| Tooltip {
            text: text.clone(),
            theme,
        })
        .into()
    }
}
