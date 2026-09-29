# proj

A project launcher that runs in the background: press a global shortcut, fuzzy-search your
projects, hit enter to open one in your editor. Built with [gpui](https://crates.io/crates/gpui).

## Usage

```sh
cargo build --release
proj            # start the launcher in the background
```

Press **ctrl+alt+space** (**alt+space** on macOS) to toggle the dialog.

On first launch the dialog opens by itself. It asks which editor to use (it lists the ones it
finds installed, plus "Other…" to pick any program, or "No editor" to use the file manager),
then opens a folder picker where you can select one or more projects at once.

| Key                     | Action                                     |
| ----------------------- | ------------------------------------------ |
| type                    | fuzzy filter by name, then by path         |
| ↑ / ↓, ctrl-p / ctrl-n  | move selection                             |
| enter                   | open in editor                             |
| shift-enter             | open a terminal there (Windows Terminal if installed) |
| alt-enter               | open with… (choose an editor for this project) |
| tab                     | mark projects to open together in one window |
| ctrl-enter              | open folder in file manager                |
| ctrl-s                  | pin / unpin (pinned projects stay on top)  |
| ctrl-shift-c            | copy the project path                      |
| ctrl-o                  | add projects (multi-select folder picker)  |
| ctrl-e                  | change editor                              |
| ctrl-d, shift-delete    | remove project                             |
| esc, clicking elsewhere | close                                      |
| ctrl-q                  | quit the launcher                          |

Type `>` to list commands: start on login, add projects, change editor, open the config
file, quit.

### Opening projects together

Press **tab** on a project to mark it; marks stay while you change the search. Then press
**enter** to open all the marked projects in one editor window. For example: type `inter`,
tab, type `shared`, tab, enter. That runs `zed interactive-v2 shared-sdk`; VS Code works the same way.
Esc clears the marks.

The combination is remembered as its own entry, **interactive-v2 + shared-sdk**, with its own
history, pin and editor list, so next time you just search for it. Removing it with ctrl-d
forgets the combination, not the projects. Folders are passed in the order you marked them.

### Per-project editors

The global editor (ctrl-e) opens everything by default. For a single project, press
**alt-enter** to open it with any editor. In that list:

- **ctrl-s** adds or removes an editor for the project. The first time you add one, the
  global editor is kept alongside it.
- **ctrl-enter** makes the selected editor the project's default.

What enter does on a project then depends on its editor list:

| Project editors | Enter                                              |
| --------------- | -------------------------------------------------- |
| none            | opens with the global editor                       |
| one             | opens with that editor                             |
| two or more     | asks which, with the first one selected (enter twice = default) |

Example: a WPF app that offers Zed and Visual Studio, and `tomi-go` that offers Zed and VS Code.
Visual Studio is found automatically and gets the project's `.sln`/`.slnx` instead of the folder.
The lists live in `projects.toml` under `[editors]`.

Each row shows the current git branch (read from `.git/HEAD`) and when you last opened it.
Projects that share a folder name get their parent folder added, e.g. `app (client)`.

Typing or pasting a folder path (`C:\…`, `~/…`, `/…`) shows an **Add project** row; press enter.

CLI:

```sh
proj add [PATH]     # add a project (default: current directory)
proj remove PATH    # remove / hide a project
proj list           # list projects
proj paths          # where the config and database live
proj autostart [on|off]  # start proj when you log in
```

## Configuration

`config.toml` (see `proj paths`) is created on first run with comments explaining each option:

```toml
hotkey = "ctrl+alt+space"
scan_dirs = []          # optional: list every sub-folder of these folders as projects
scan_depth = 1          # >1 descends into non-git folders
editor = "zed"          # set from the launcher; "" = file manager
editor_args = []        # e.g. ["--new-window"]
```

Picking an editor in the launcher updates only the `editor` line; the rest of the file,
comments included, is left alone.

Config and folder scans are re-read every time the dialog opens, so changes apply immediately.
Changing the hotkey is the exception: that needs a restart.
Manually added projects, hidden projects and open history live in `projects.toml`.

## Memory

The dialog window is destroyed when it closes. On Windows the process then trims its working
set, so it sits at about 1–3 MB of working set while idle. Committed memory stays higher,
around 35 MB before the first open and about 70 MB after it, mostly gpui's GPU and font state.

## Start on login

Type `>` and pick **Start on login**, or run `proj autostart on`. This uses the registry Run
key on Windows, a LaunchAgent on macOS, and `~/.config/autostart` on Linux.

## Platform notes

Only tested on Windows so far. Global hotkeys don't work on Linux Wayland (a limitation of
`global-hotkey`); X11 is fine. On macOS the app shows a Dock icon, because gpui 0.2 doesn't
expose the accessory activation policy.
