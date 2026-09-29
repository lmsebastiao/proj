//! Palette actions and their key bindings.

use gpui::{App, KeyBinding, actions};

use crate::input;

actions!(
    palette,
    [
        SelectNext,
        SelectPrev,
        Confirm,
        Reveal,
        OpenTerminal,
        OpenWithMenu,
        OpenRemote,
        ToggleMark,
        TogglePin,
        Rename,
        CopyPath,
        Remove,
        AddProjects,
        ChooseEditor,
        Dismiss,
        QuitApp
    ]
);

pub fn bind_keys(cx: &mut App) {
    input::bind_keys(cx);
    let ctx = Some("Palette");
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("ctrl-n", SelectNext, ctx),
        KeyBinding::new("up", SelectPrev, ctx),
        KeyBinding::new("ctrl-p", SelectPrev, ctx),
        KeyBinding::new("shift-tab", SelectPrev, ctx),
        KeyBinding::new("enter", Confirm, ctx),
        KeyBinding::new("secondary-enter", Reveal, ctx),
        KeyBinding::new("shift-enter", OpenTerminal, ctx),
        KeyBinding::new("alt-enter", OpenWithMenu, ctx),
        KeyBinding::new("secondary-g", OpenRemote, ctx),
        KeyBinding::new("tab", ToggleMark, ctx),
        KeyBinding::new("secondary-s", TogglePin, ctx),
        KeyBinding::new("f2", Rename, ctx),
        KeyBinding::new("secondary-shift-c", CopyPath, ctx),
        KeyBinding::new("secondary-d", Remove, ctx),
        KeyBinding::new("shift-delete", Remove, ctx),
        KeyBinding::new("secondary-o", AddProjects, ctx),
        KeyBinding::new("secondary-e", ChooseEditor, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        KeyBinding::new("secondary-q", QuitApp, ctx),
    ]);
    cx.on_action(|_: &QuitApp, cx| cx.quit());
}
