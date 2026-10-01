//! Palette actions and their key bindings.
//!
//! Enter and its variants open things; other actions are ctrl (cmd on macOS)
//! plus a letter named after them, or the key Windows uses for them (F2,
//! shift-delete). The labels in `shortcuts` and the banners
//! in `render` name these keys, so change them together.

use gpui::{App, KeyBinding, actions};

use crate::input;

actions!(
    palette,
    [
        SelectNext,
        SelectPrev,
        MoveDown,
        MoveUp,
        Confirm,
        ConfirmSecondary,
        ShowInFileManager,
        OpenTerminal,
        OpenWithMenu,
        OpenRemote,
        ToggleMark,
        ToggleMarkUp,
        ShowActions,
        ShowCommands,
        CopyPath,
        AddProjects,
        ToggleShortcuts,
        Dismiss,
        QuitApp,
        RemoveItem,
        RenameItem,
        OpenConfig,
        CloseWindow,
        UndoRemove,
        SelectPageDown,
        SelectPageUp
    ]
);

/// The project actions' ctrl keys also work with alt (see `bind_keys`).
pub(super) const ALT_ACTIONS: bool = !cfg!(target_os = "macos");

pub fn bind_keys(cx: &mut App) {
    input::bind_keys(cx);
    let ctx = Some("Palette");
    cx.bind_keys([
        // Moving
        KeyBinding::new("down", SelectNext, ctx),
        KeyBinding::new("up", SelectPrev, ctx),
        KeyBinding::new("pagedown", SelectPageDown, ctx),
        KeyBinding::new("pageup", SelectPageUp, ctx),
        // Home and end move the text cursor, and with an empty search, the
        // selection (see `render`).
        // Opening
        KeyBinding::new("enter", Confirm, ctx),
        // Open with another editor, or set the project's default editor.
        KeyBinding::new("alt-enter", OpenWithMenu, ctx),
        // The second thing a row does, as in PowerToys' Command Palette: on a
        // project, open with…; in the Open-with list, make the highlighted
        // editor the project's default.
        KeyBinding::new("secondary-enter", ConfirmSecondary, ctx),
        KeyBinding::new("secondary-t", OpenTerminal, ctx),
        KeyBinding::new("secondary-e", ShowInFileManager, ctx),
        KeyBinding::new("secondary-g", OpenRemote, ctx),
        // Mark projects to open together, then move down / up (other lists: just move).
        KeyBinding::new("tab", ToggleMark, ctx),
        KeyBinding::new("shift-tab", ToggleMarkUp, ctx),
        // Everything else for the project (rename, tags, remove…), also on the
        // row's icons. Shift-F10 and the menu key open context menus elsewhere.
        KeyBinding::new("secondary-k", ShowActions, ctx),
        KeyBinding::new("shift-f10", ShowActions, ctx),
        KeyBinding::new("menu", ShowActions, ctx),
        // Its commands to run (package.json scripts and added ones).
        KeyBinding::new("secondary-r", ShowCommands, ctx),
        // The search box copies its selected text instead, when there is some.
        KeyBinding::new("secondary-c", CopyPath, ctx),
        KeyBinding::new("secondary-o", AddProjects, ctx),
        // The palette itself
        KeyBinding::new("f1", ToggleShortcuts, ctx),
        KeyBinding::new("escape", Dismiss, ctx),
        KeyBinding::new("secondary-q", QuitApp, ctx),
        // Take the project off the list (ctrl-z puts it back), or a command
        // added to its actions menu out of it. Shift-delete, as in browsers'
        // address bar suggestions.
        KeyBinding::new("shift-delete", RemoveItem, ctx),
        // As in Explorer.
        KeyBinding::new("f2", RenameItem, ctx),
        KeyBinding::new("secondary-,", OpenConfig, ctx),
        // Put back what was just removed from the list.
        KeyBinding::new("secondary-z", UndoRemove, ctx),
        // The window switcher: close the highlighted window.
        KeyBinding::new("secondary-w", CloseWindow, ctx),
        // The window switcher is used with alt held down.
        KeyBinding::new("alt-escape", Dismiss, ctx),
        // Also move a group's ticked folder in the order they open in.
        KeyBinding::new("alt-down", MoveDown, ctx),
        KeyBinding::new("alt-up", MoveUp, ctx),
        // Into a project's windows, and back out.
        KeyBinding::new("alt-right", input::Right, ctx),
        KeyBinding::new("alt-left", input::Left, ctx),
        KeyBinding::new("alt-secondary-w", CloseWindow, ctx),
    ]);
    // The highlighted project's actions also come with alt, which is still
    // down right after alt+space: no reaching over for ctrl. (Alt+1…9 too,
    // in `open_number`.) The rest stay on ctrl only: ctrl-z, ctrl-o, ctrl-q
    // and ctrl-, mean what they mean everywhere. Not on macOS, where
    // option+letter types a character.
    if ALT_ACTIONS {
        cx.bind_keys([
            KeyBinding::new("alt-k", ShowActions, ctx),
            KeyBinding::new("alt-r", ShowCommands, ctx),
            KeyBinding::new("alt-t", OpenTerminal, ctx),
            KeyBinding::new("alt-e", ShowInFileManager, ctx),
            KeyBinding::new("alt-g", OpenRemote, ctx),
            KeyBinding::new("alt-c", CopyPath, ctx),
        ]);
    }
    cx.on_action(|_: &QuitApp, cx| cx.quit());
}
