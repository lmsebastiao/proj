# proj

A project launcher that runs in the background: press a global shortcut, fuzzy-search your
projects, hit enter to open one in your editor. Built with [gpui](https://crates.io/crates/gpui).

## Usage

```sh
cargo build --release
proj            # start the launcher in the background
```

Press **ctrl+alt+space** (**alt+space** on macOS) to toggle the dialog, or click the tray icon.
Right-click the tray icon for: open, start on login, open config file, check for updates, quit
(Windows and macOS; Linux has no tray icon). `hotkey` can also be a list:
`["ctrl+alt+space", "f8"]`.

On first launch the dialog opens by itself. It asks which editor to use (it lists the ones it
finds installed, plus "Other…" to pick any program, or "No editor" to use the file manager),
then opens a folder picker where you can select one or more projects at once.

| Key                     | Action                                     |
| ----------------------- | ------------------------------------------ |
| type                    | fuzzy filter by name, then by path         |
| ↑ / ↓                   | move selection                             |
| enter                   | open in editor                             |
| ctrl-t                  | open a terminal there (Windows Terminal if installed) |
| → / ←                   | browse into a project / back up (see below) |
| alt-enter               | open with… (another editor once, or set the project's default) |
| tab / shift-tab         | mark projects to open together, moving down / up |
| ctrl-k, shift-f10, menu key | actions for the project: pin, rename, remove, and the ones below |
| ctrl-e                  | show in Explorer (Finder on macOS, the file manager on Linux) |
| ctrl-g                  | open the repository page (from the git `origin` remote) |
| ctrl-c                  | copy the project path (text, if some is selected) |
| ctrl-o                  | add projects (multi-select folder picker)  |
| f1                      | list every shortcut (click one to run it)  |
| esc, clicking elsewhere | close                                      |
| ctrl-q                  | quit the launcher                          |

The footer shows the most common keys for the current list. **f1**, or clicking **all keys**
in the footer, opens a dropdown with all of them; clicking one runs it.

**ctrl-k** (or shift-f10, or the menu key) lists everything you can do with the highlighted
project: open with, show in Explorer, terminal, pin, rename, copy the path, open the
repository page, remove. Type to filter it and press enter; the project's name and folder
show above the list. The highlighted
row, and any row under the mouse, also has icons for rename, remove, **⋯** (the same list)
and pin. The remove icon asks for a second click. Pinned projects stay on top and keep their
pin icon showing. Renaming to an empty name goes back to the folder name.
Removing only takes a project off the list; the folder isn't touched.

Type `>` to list commands: start on login, add projects, change the default editor, theme,
open the config file, check for updates / install update (installed copies), quit.
Type `@` to list your open editor windows instead, the same list as the switcher (below):
keep typing to filter it, enter switches, esc goes back to the projects.

The dialog follows Windows' light/dark app setting, switching live when Windows does. `>`
**Theme** goes through system → light → dark and saves the choice as `theme` in config.toml.

### Browsing inside a project

Press **→** on a project to list its files and folders; **→** on a folder goes deeper,
**←** goes back up (and back to the project list from the top), **esc** returns to the list.
Typing filters the current folder. **Enter** opens a file in the project's editor, inside the
project's window (`zed <project> <file>`), or opens a subfolder as its own workspace.
**ctrl-e** shows it in the file manager, **ctrl-t** opens a terminal there, and **ctrl-c**
copies its path. Nothing you browse is added to the project list.

→/← only browse when the text cursor is at the end/start of the search box, so they still
move the cursor while you edit a search.

### Switching between open projects

Hold **alt** and tap the key left of 1 (**\\** on Portuguese keyboards, **`** on US ones) to
switch between your open editor windows, like Alt+Tab but only for editors. Keep tapping to
move down the list; add **shift** to go back up. Let go of alt and that window comes to the
front. The list shows as soon as you press the key, like Alt+Tab; a quick tap switches to the
editor window you used before. **alt-esc** cancels.

The list keeps its order: windows stay where they first appeared, new ones are added at the
end, and switching doesn't move anything. The highlight starts on the window you used before
the current one.

Press **ctrl+alt** and the same key (ctrl+alt+\\ on Portuguese keyboards) to search instead:
the same list, but it stays open, typing filters it, **enter** switches and **esc** closes.
While holding the switcher open, adding ctrl turns it into a search. From the project search,
typing `@` gets you the same list.

Each window is listed under its project, with the branch and the editor. proj works out the
project from the window title: the folder name (Zed, VS Code, Cursor…), a Zed workspace's
folder list, or the solution name (Visual Studio). Windows it can't match are listed by their
title. Projects with a window open get an **open** badge in the normal list.

`switch_hotkey` and `switch_search_hotkey` in config.toml set other shortcuts, e.g. `"alt+q"`.
Set one to `""` to turn it off. Windows only.

### Opening projects together

Press **tab** on a project to mark it; marks stay while you change the search. Then press
**enter** to open all the marked projects in one editor window. For example: type `inter`,
tab, type `shared`, tab, enter. That runs `zed interactive-v2 shared-sdk`; VS Code works the same way.
Esc clears the marks.

The combination is remembered as its own entry, **interactive-v2 + shared-sdk**, with its own
history, pin and editor list, so next time you just search for it. Removing it (ctrl-k or the bin icon)
forgets the combination, not the projects. Folders are passed in the order you marked them.
Rename it (ctrl-k or the pencil icon) to give it a shorter name; search still finds it by its
folder names too.

### Per-project editors

Enter opens a project in its own default editor if it has one, and otherwise in the default
editor for all projects. Rows of projects with their own default show its name.

- **Just this once:** press **alt-enter** on a project, pick an editor, press **enter**.
  Nothing is saved, so next time enter uses the usual editor again.
- **This project's default:** in the same alt-enter list, press **ctrl-enter** on an editor
  (on **Other…**, it asks for a program first). Press ctrl-enter on it again to go back to
  the default for all projects.
- **All projects:** type `>` and pick **Change the default editor**. Projects with their own
  default keep it.

For example, a WPF app can default to Visual Studio while everything else opens in Zed.
Visual Studio is found automatically and gets the project's `.sln`/`.slnx` instead of the
folder. Project defaults live in `projects.toml` under `[editors]`.

Each row shows the current git branch (read from `.git/HEAD`) and when you last opened it.
Projects that share a folder name get their parent folder added, e.g. `app (client)`.

Typing or pasting a folder path (`C:\…`, `~/…`, `/…`) shows an **Add project** row; press enter.

Pasting a git URL (`https://…`, `ssh://…`, `git@host:owner/repo.git`) shows a **Clone** row.
Enter runs `git clone` into the first `scan_dirs` folder (or asks for a folder if there are none),
then opens the new project. The dialog stays open while it clones; if you close it, the clone
still finishes and is listed next time. Private repositories work the way they do for `git` on
your machine (SSH keys, Git Credential Manager).

CLI:

```sh
proj open QUERY     # open the best match, as if you'd typed QUERY and pressed enter
proj add [PATH]     # add a project (default: current directory)
proj remove PATH    # remove / hide a project
proj list           # list projects
proj paths          # where the config and database live
proj autostart [on|off]  # start proj when you log in
proj path [add|remove]   # put proj's folder on your user PATH (Windows)
proj update         # install the latest release, if it is newer (see Updates)
proj version        # show the installed version
```

## Configuration

`config.toml` (see `proj paths`) is created on first run with comments explaining each option:

```toml
hotkey = "ctrl+alt+space"
# switch_hotkey = "alt+q"  # window switcher; default alt + the key left of 1, "" = off
# switch_search_hotkey = "ctrl+alt+q"  # searching switcher; default ctrl+alt + that key
scan_dirs = []          # optional: list every sub-folder of these folders as projects
scan_depth = 1          # >1 descends into non-git folders
check_for_updates = true  # look for a new release about once a day
theme = "system"        # or "light" / "dark"; also set with > Theme
editor = "zed"          # set from the launcher; "" = file manager
editor_args = []        # e.g. ["--new-window"]
```

Picking an editor in the launcher updates only the `editor` line; the rest of the file,
comments included, is left alone.

Config and folder scans are re-read every time the dialog opens, so changes apply immediately.
That includes `hotkey`: a new shortcut starts working once the dialog has opened, by the old
shortcut or the tray icon. If none of the new shortcuts can be registered, the old ones keep
working and the dialog says why.
Manually added projects, hidden projects, names and open history live in `projects.toml`.

## Windows installer

```powershell
.\scripts\build-installer.ps1              # cargo build --release + package
.\scripts\build-installer.ps1 -SkipBuild   # package the last release build
```

This produces `dist\proj-setup-<version>.exe` (version from `Cargo.toml`). The script uses
`makensis` if NSIS is installed; otherwise it downloads the pinned portable NSIS release once into
`%LOCALAPPDATA%\proj-build`, checking its SHA-256. Nothing is installed system-wide.

The installer is per user, so it needs no admin rights:

- installs to `%LOCALAPPDATA%\Programs\proj` and adds that folder to the user PATH, so `proj add .`
  works in new terminals
- adds a Start menu shortcut and an "Apps & features" entry
- offers "Start proj when I log in" and "Start proj now" on the finish page
- stops a running proj before upgrading
- the uninstaller removes the PATH entry and start-on-login, but keeps your settings in
  `%APPDATA%\proj`
- supports silent install and uninstall: `proj-setup-x.y.z.exe /S`, `uninstall.exe /S`

The PATH is edited by `proj path add/remove`, not by NSIS. NSIS strings are limited to 1024
characters, which would truncate a long PATH.

## Updates

A copy installed with the installer looks for a newer
[GitHub release](https://github.com/lmsebastiao/proj/releases) a minute after it starts and
then about once a day. Set `check_for_updates = false` to turn that off. When a new version
is out, the tray menu's **Check for updates** item becomes **Install update x.y.z**, and so does
the same command in the `>` list. Either one checks now when clicked, or once an update is
found, downloads the installer and runs it silently
(`proj-setup-x.y.z.exe /S /RELAUNCH /D=<install folder>`). The installer replaces proj.exe
and starts the new version. Nothing is installed without that click, or without running
`proj update`.

Copies built with `cargo build` don't update themselves: they have no `uninstall.exe` next to
them, so neither the tray nor the `>` list has the update command, and `proj update` only says
whether a newer version exists.

## Releasing

GitHub Actions runs `cargo fmt --check`, clippy and the tests on every push to `main` and every
pull request (`.github/workflows/ci.yml`). To publish a version:

1. Bump `version` in `Cargo.toml` (e.g. `0.3.0`), run `cargo check` so `Cargo.lock` follows,
   commit and push to `main`.
2. Run the **Release** workflow: on GitHub, Actions → Release → Run workflow (branch `main`),
   or `gh workflow run release.yml`.

`.github/workflows/release.yml` refuses to run from any branch other than `main`, or when the
version in `Cargo.toml` is already tagged. It then runs `scripts/build-installer.ps1` and
creates the `v0.3.0` tag and a release with `proj-setup-0.3.0.exe` and `SHA256SUMS.txt`, with
notes generated from the commits. You don't create the tag yourself. Installed copies pick the
release up on their next check.

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
