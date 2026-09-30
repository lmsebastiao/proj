//! Palette actions and their key bindings.
//!
//! Enter and its variants open things; other actions are ctrl (cmd on macOS)
//! plus a letter named after them. The labels in `shortcuts` and the banners
//! in `render` name these keys, so change them together.

use gpui::{App, KeyBinding, actions};

use crate::input;

actions!(
    palette,
    [
        SelectNext,
        SelectPrev,
        Confirm,
        AlwaysOpenWith,
        ShowInFileManager,
        OpenTerminal,
        OpenWithMenu,
        OpenRemote,
        ToggleMark,
        ToggleMarkUp,
        TogglePin,
        Rename,
        CopyPath,
        Remove,
        AddProjects,
        ToggleShortcuts,
        Dismiss,
        QuitApp
    ]
);

pub fn bind_keys(cx: &mut App) {
    input::bind_keys(cx);
    let ctx = Some("Palette");
    cx.bind_keys([
        // Moving
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("up", SelectPrev, ctx),
        // Opening
        KeyBinding::new("enter", Confirm, ctx),
        // In the Open-with list: always open this project with the selected editor.
        KeyBinding::new("secondary-enter", AlwaysOpenWith, ctx),
        KeyBinding::new("secondary-w", OpenWithMenu, ctx),
        KeyBinding::new("secondary-t", OpenTerminal, ctx),
        KeyBinding::new("secondary-e", ShowInFileManager, ctx),
        KeyBinding::new("secondary-g", OpenRemote, ctx),
        // Mark projects to open together, then move down / up (other lists: just move).
        KeyBinding::new("tab", ToggleMark, ctx),
        KeyBinding::new("shift-tab", ToggleMarkUp, ctx),
        // Changing the list
        // Pin; in the Open-with list, add/remove the editor from the pick list.
        KeyBinding::new("secondary-p", TogglePin, ctx),
        KeyBinding::new("f2", Rename, ctx),
        // The search box copies its selected text instead, when there is some.
        KeyBinding::new("secondary-c", CopyPath, ctx),
        KeyBinding::new("shift-delete", Remove, ctx),
        KeyBinding::new("secondary-o", AddProjects, ctx),
        // The palette itself
        KeyBinding::new("f1", ToggleShortcuts, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        KeyBinding::new("secondary-q", QuitApp, ctx),
        // The window switcher is used with alt held down.
        KeyBinding::new("alt-escape", Dismiss, ctx),
        KeyBinding::new("alt-down", SelectNext, ctx),
        KeyBinding::new("alt-up", SelectPrev, ctx),
    ]);
    cx.on_action(|_: &QuitApp, cx| cx.quit());
}
