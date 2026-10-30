# proj

A project launcher that runs in the background: press a global shortcut, fuzzy-search your
projects, hit enter to open one in your editor. Built with [gpui](https://crates.io/crates/gpui).

## Workspace

`crates/proj` contains only the `proj` entry point and Windows icon build script.
`crates/proj-core` contains independent fuzzy matching and ranking; `crates/proj-git`
reads Git metadata and status; `crates/proj-projects` owns project discovery, settings,
persistence, templates, tasks and opening; `crates/proj-cli` owns CLI commands, updates,
autostart and shared OS/platform integration; `crates/proj-ui` owns the GPUI launcher,
search palette, input and window switcher. The CLI also depends on GPUI because the shared
platform integration exposes window operations. Dependencies run from core/Git through
projects and CLI to UI; none of these crates depends on the binary.

From the repository root, plain `cargo build --release` builds `proj`;
`cargo check --workspace` and `cargo test --workspace` include every crate. Icons, the
installer and release scripts remain at the repository root. The binary and CLI share
`[workspace.package].version` so the updater reports the same version as the binary.

## Usage

```sh
cargo build --release  # from the repository root; builds the proj binary
proj            # start the launcher in the background
```

Press **alt+space** to toggle the dialog, or click the tray icon.
Right-click the tray icon for: open, start on login, open config file, check for updates, quit
(Windows and macOS; Linux has no tray icon). `hotkey` can also be a list:
`["alt+space", "f8"]`. On Windows, alt+space normally opens a window's system menu, and
PowerToys Run uses it too; if another app has it, the dialog says so and you can set another.

On first launch the dialog opens by itself. It asks which editor to use (it lists the ones it
finds installed, plus "Other…" to pick any program, or "No editor" to use the file manager),
then suggests existing common project folders (such as `~/repos`) to watch with one click,
or lets you choose your own. If none are found, it opens a folder picker. Their direct
subfolders appear as projects automatically, including folders created later. You can
still add individual projects (ctrl-o). Use `> Discover projects in folders…` to watch
more folders later, or `> Manage watched folders…` to see their project counts, add another,
set a root's depth or excluded folders, or stop watching one. On that page, Enter on a root
sets its depth (leave empty to inherit the global depth); Ctrl/Cmd+Enter edits exclusions.
Stopping a watch never deletes its folder. Nothing outside the chosen folders is scanned.

| Key                     | Action                                     |
| ----------------------- | ------------------------------------------ |
| type                    | fuzzy filter by name, then by path         |
| ↑ / ↓                   | move selection                             |
| pgup / pgdn, home / end | a page at a time; the first / last (with nothing typed) |
| ctrl-1 … 9, alt-1 … 9   | open the project in that place (hold ctrl or alt a moment to see the numbers) |
| enter                   | open in editor                             |
| ctrl-enter, alt-enter   | open with… (another editor once, or set the project's default) |
| ctrl-k / alt-k, shift-f10, menu key, right-click | actions for the project: rename, remove, and the ones below |
| ctrl-r / alt-r          | its commands to run: package.json scripts and your own (see Commands) |
| f2                      | rename                                     |
| shift-delete            | remove from the list (the folder stays)    |
| ctrl-z                  | put back the project just removed          |
| ctrl-t / alt-t          | open a terminal there (Windows Terminal if installed) |
| → / ←                   | browse into a project / back up (see below) |
| backspace               | with nothing typed: back a page (out of a folder, the Open-with list…) |
| tab / shift-tab         | mark projects to open together once, moving down / up (see Groups) |
| ctrl-e / alt-e          | show in Explorer (Finder on macOS, the file manager on Linux) |
| ctrl-g / alt-g          | open the repository page (from the git `origin` remote) |
| ctrl-c / alt-c          | copy the project path (ctrl-c: the text, if some is selected) |
| ctrl-o                  | add projects (multi-select folder picker)  |
| ctrl-,                  | open the config file                       |
| f1                      | list every shortcut (↑/↓ and enter, or click one, to run it) |
| esc, clicking elsewhere | close                                      |
| ctrl-q                  | quit the launcher                          |

The keys for the highlighted project's actions work with **alt** as well as ctrl (not on
macOS, where option types characters). Alt is still down right after alt+space, so
alt+space then alt-k, without letting go, opens the actions menu. Ctrl-z, ctrl-o, ctrl-q and
ctrl-, stay on ctrl only: they mean the same everywhere.

The footer has buttons for the current list's main two keys, like PowerToys' Command
Palette: **Open ↵** and **Actions ctrl k** on the projects. Its left side says how many
projects there are, or what just happened. Problems show above it in full, in red, until you
type. **f1**, or the **?** button, opens a dropdown with every key. While it's open, ↑/↓ move
through it instead of the list and enter runs the highlighted key; clicking one runs it too.
Esc or typing closes it.

**ctrl-k** (or alt-k, handy right after alt+space; shift-f10, the menu key, or right-clicking a row) opens a menu over the list,
at the bottom right, with everything you can do with the highlighted project: open with,
show in Explorer, terminal, run a command (the ctrl-r menu, see Commands), rename,
open the repository page, its pull requests or its CI runs, copy the clone URL, change
what its git site runs, copy the path, start a new project from it, pin/unpin, remove. Each shows its own key, if it has one. Type to filter it and press
enter; esc, ctrl-k (or alt-k) again, backspace with nothing typed or clicking outside closes it. The
highlighted row, and any row under the mouse, also has a **⋯** icon that opens the same menu.

With nothing typed, the projects with an editor window open come first: the one you were
in when you pressed the shortcut, then the others, the one used last first. Then pinned
projects, followed by the rest, the one opened last first, under a line. Pin or unpin one
from its actions (ctrl-k) so a favorite is near the top whenever the search is empty.

On Windows, when you open a new local folder in Zed or Zed Preview, proj can offer to
add it in a small, non-focusing popup for seven seconds. Choose **Add** or **Dismiss**;
no folder is added without your confirmation. Multi-folder Zed windows can be saved as
workspaces (their folders open together as one entry). Already listed or hidden folders
aren't offered again. The prompt waits while the search dialog is open. Detection uses
Zed's local workspace database, so it skips ambiguous titles, remote folders, other
editors, and editor windows already open when proj starts.

A listed project or workspace whose folder is deleted disappears automatically the
next time the project list is loaded. Folders on inaccessible or disconnected drives
are hidden until they become available again, rather than forgotten.

Searching uses fzf's way of matching (through [nucleo](https://github.com/helix-editor/nucleo),
as Helix and Zed do): letters at the start of words and next to each other count most, and
each word you type matches on its own, in any order (`sdk shared` finds `shared-sdk`).
The best matches come first, and among matches about as good, the projects
you open more, and more lately: each of the last ten opens counts, less the older it is. So
`examp` puts the `example-v2` you opened minutes ago above the `example` from an hour
ago, though its name is longer. A group is an entry of its own, with its own opens.

**f2** renames a project; renaming to an empty name goes back to the folder name.
**shift-delete** removes it, only from the list, as the folder isn't touched. The footer
then offers **Undo** (or ctrl-z) for a few seconds. A row says **watched**, **added**, or
**workspace** so you know why it is listed; hovering the right-hand details says which
watched folder it came from. Removing a watched project hides it on later scans.

Every row starts with an icon: the editor the project opens in, a folder or file while
browsing, and the program in editor lists and window cards. On the pages you reach from the project
list (Open with, browsing, rename…), the search bar starts with a back button and the
page's name; backspace with nothing typed goes back too. Copying a path says **Copied** for
a moment before the dialog closes.

If the dialog closes without opening anything (esc, the shortcut again, or a click
elsewhere), opening it again within 30 seconds brings back what you'd typed, selected so that
typing replaces it.

Type `>` to list commands: start on login, discover projects in folders, add individual projects, new project from a template, change
the default editor, theme, open the config file,
check for updates / install update (installed copies), reset, quit. **Reset proj…**
requires typing `RESET` and pressing enter. It clears proj's config, watched and added
projects, hidden entries, custom names/editors/commands, pins, and usage history, and turns
off start on login. It never deletes project folders or Git data.
Type `$` and part of a file's name to find it in any of your projects, as ctrl-p does in an
editor (see Finding files).

The dialog follows Windows' light/dark app setting, switching live when Windows does. `>`
**Theme** goes through system → light → dark and saves the choice as `theme` in config.toml.

### Browsing inside a project

Press **→** on a project to list its files and folders; **→** on a folder goes deeper,
**←** (or backspace, with nothing typed) goes back up (and back to the project list from the
top), **esc** returns to the list.
Typing filters the current folder. **Enter** opens a file in the project's editor, inside the
project's window (`zed <project> <file>`), or opens a subfolder as its own workspace.
**ctrl-e** shows it in the file manager, **ctrl-t** opens a terminal there, and **ctrl-c**
copies its path. Nothing you browse is added to the project list.

### Finding files

Type **`$`** and then part of a file's name (`$button`) to search every listed project's files
at once. Words before the name narrow it by the folders it's in, the project's included, in
any order, split by spaces or either slash: `$palette files`, `$src/main`, `$web\src\index`. A folder's
name alone (`$components`) lists what's in it, after any files named like it. Files whose
name matches come first, the ones nearer the top of their project before deeper ones. Enter
opens the file in its project's editor and window, as in browsing; **ctrl-e**, **ctrl-t** and
**ctrl-c** work there too.

Repositories list what git does, so what `.gitignore` leaves out (build output, packages)
stays out. Other folders leave out `node_modules`, `target` and the like. Submodules, and
repositories inside a project that were never added to it, are searched as part of it. A
project inside another one that's in your list too (a submodule, or just a folder) keeps its
files to itself, so each file shows once, under the innermost project. The lists are read
in the background the first time you type `$`, and kept, then read again when a minute old.
The best 200 matches show.

→/← only browse when the text cursor is at the end/start of the search box, so they still
move the cursor while you edit a search.

### Switching between editor windows

Hold **alt** and tap **\\** to open an Alt+Tab-style overlay of your open editor window
cards. It's the \\ key wherever your layout has it: left of 1 on Portuguese keyboards,
above enter on US ones. Cards appear in recent-window order, front to back, with the
previously used editor selected first. Keep tapping **\\** to cycle forward; hold
**shift** while tapping to cycle backward. Release **alt** to bring the selected window
to the front, or press **esc** to cancel. A quick tap returns to the previous editor.
While holding **alt**, press **1** through **9** to switch straight to a numbered card;
numbers beyond the first nine windows do nothing.

Every editor window has its own card, even when two windows show the same project.
While holding **alt**, press **ctrl+w** (or **ctrl+alt+w**) to close the selected
window, as its close button would, so the editor can still ask about unsaved changes.
Each card also has an X button to close its window without switching to it.

Each card shows its window title and, when available, the project's image. For public
repositories on gitlab.com, gitea.com or codeberg.org, proj fetches the repository
avatar in the background and prefers it over local images. It also checks `.github/`
and `.gitlab/`, then the project root, `assets/`, and `public/` for `logo` or `icon`
files (`.png`, `.webp`, `.jpg`, `.jpeg`, `.svg`), then `public/favicon.png`,
`public/favicon.ico`, and `favicon.ico`. If none is available, it keeps the editor
icon. GitHub does not expose a distinct repository avatar, so proj uses the local
image paths there. proj works out the project from the window title: the folder
name (Zed, VS Code, Cursor…), a Zed workspace's folder list, or the solution
name (Visual Studio).
Windows it can't match confidently still appear by title; you can switch to them there.
Projects with a window open get a green dot after their name in the normal list. **Enter**
switches to its window when there's one match; when there are several, it shows their titles
and editors so you can choose one or explicitly launch the project again. **alt-↵** still
opens it in an editor without choosing a matched window.

`switch_hotkey` in config.toml sets another shortcut, e.g. `"alt+q"`, or `""` turns the
switcher off. Windows and macOS; on macOS proj needs to be allowed under System Settings ›
Privacy & Security › Accessibility (it asks the first time), and lists the apps with windows
on screen, so not ones with only minimized windows or windows on other Spaces.

### Groups: opening projects together

Several projects can open in one editor window: `zed example-v2 sample-sdk`, and the
same for VS Code. Folders are passed in the order they were ticked.

**Once:** press **tab** on a project to mark it (marks stay while you change the search), and
**enter** opens all the marked ones together. For example: type `exam`, tab, type `sample`,
tab, enter. Nothing is saved. Esc clears the marks.

**To keep:** ctrl-k on a project → **Open together with…** opens a page listing your projects
with that one ticked. **tab** or **space** ticks or unticks the highlighted project, and
**alt-↑/↓** moves it earlier or later in the order (its number shows it). **enter** opens the
ticked ones together once; **ctrl-enter** saves them as a group, a row of its own in the list,
with its own history, editor and commands, so next time you just search for it.
Ticking the same projects in another order finds the same group.

A group is named after its first folder, `example-v2 Workspace`, as everything it does goes
by that folder (its repository pages, terminal, file manager). Groups that start with the same
folder add their others: `example-v2 Workspace (sample-sdk)`. So searching `sample` finds the
`sample-sdk` project first, above the groups it's in, which follow since their folders still
match. A group's row has a folder icon with how many folders it has, and its folders' names under
its name, each with a green dot when that project has a window of its own open. Its branches
show on the right; hovering them lists each folder's branch and changes. ctrl-k on a group →
**Folders…** opens the same page with its folders ticked, to add, take out or reorder them;
enter saves, keeping its name and history. Rename it (f2) to give it a shorter name;
search still finds it by its folders' names. Removing it (shift-delete) forgets the group, not
the projects.

### Per-project editors

Enter opens a project in its own default editor if it has one, and otherwise in the default
editor for all projects. Rows of projects with their own default show its name.

- **Just this once:** press **ctrl-enter** (or alt-enter) on a project, pick an editor, press **enter**.
  Nothing is saved, so next time enter uses the usual editor again.
- **This project's default:** in the same list, press **ctrl-enter** on an editor
  (on **Other…**, it asks for a program first). Press ctrl-enter on it again to go back to
  the default for all projects.
- **All projects:** type `>` and pick **Change the default editor**. Projects with their own
  default keep it.

For example, a WPF app can default to Visual Studio while everything else opens in Zed.
Visual Studio is found automatically and gets the project's `.sln`/`.slnx` instead of the
folder. Project defaults live in `projects.toml` under `[editors]`.

Each row shows the current git branch (read from `.git/HEAD`) and when you last opened it.
After the branch, **●** means uncommitted changes and **↑2 ↓1** commits to push and pull:
`git status` runs in the background each time the dialog opens (at most every 10 seconds per
repository), so the counts show up a moment later, and right away the next time.
Projects that share a folder name get their parent folder added, e.g. `app (client)`.

A project you added whose folder is gone (moved, deleted, or on a drive that isn't
connected) stays listed at the end, dimmed and marked **missing**, instead of vanishing. Type
`>` and pick **Remove missing projects** to forget them all, or remove one with shift-delete.

### Commands

**ctrl-r** (or alt-r, or ctrl-k → **Run a command…**) on a project opens its commands menu,
over the list like the ctrl-k one: your saved commands first, then tasks discovered from
`package.json`, `Cargo.toml`, a Makefile or a justfile, each labeled with its source.
Package scripts use npm, pnpm, yarn or bun according to the lock file; Cargo targets,
Make targets and parameter-free Just recipes are offered without running their build tools
just to list them. Then comes **Add a command…** (e.g.
`npm run dev`). Type to filter them. Enter opens a terminal in the project's folder running
it, which stays open when it ends. The last command you ran for that project is selected
when you open the menu again, so ctrl-r then enter repeats it. **shift-delete** on an added command takes it out again.
ctrl-r again, or esc, closes the menu; ctrl-k switches to the actions.

When a clone, template copy or editor launch fails, the palette shows the attempted location
and an explicit **Retry** action. A partially created destination is never deleted or
overwritten; inspect/remove it yourself or choose another destination before retrying.
Completed background results remain available while proj is running, including when the
palette was closed before the work finished.

### Pull requests and CI

ctrl-k → **Open the pull requests** and **Open the CI runs** go to those pages of the
repository's site, from its `origin` remote: GitHub, GitLab, Gitea, Forgejo (Codeberg),
Bitbucket and Azure DevOps, by their site names (`gitlab.example.com` counts too). For a
self-hosted site with another name, proj asks what it runs the first time (GitLab, Gitea,
Forgejo…), opens the page, and saves the answer in config.toml as
`forges = { "git.example.com" = "gitlab" }`. To change it later, ctrl-k →
**Change what git.example.com runs…**, or edit that line. **Copy the clone URL** copies the remote's URL.

### New projects from templates

Type `>` and pick **New project from a template**, or ctrl-k → **New project from this one…**
on any project. On the template list, add a folder or a git URL, or remove a saved template
with **shift-delete** (its source is kept). `> Manage templates…` opens the same list;
entries are saved under `templates` in config.toml. After picking one, type the new
project's folder name; the full destination is shown. **ctrl-enter** (**cmd-enter** on macOS)
changes the parent to another watched folder or one you choose. **Enter** makes and opens it with its own git
history.
A folder that is a git repository is copied as git would commit it, so without what
`.gitignore` leaves out; other folders are copied whole, but for `node_modules`, `target` and
the like. A git URL is cloned without its history.

Typing or pasting a folder path (`C:\…`, `~/…`, `/…`) shows an **Add and open** row: enter adds
it to the list and opens it in the default editor; ctrl-enter (cmd-enter on macOS) just opens it.
A file path shows an **Open** row: the file opens in the editor of the listed project it's in (so
it lands in that project's window), or in the default editor.

Pasting a git URL (`https://…`, `ssh://…`, `git@host:owner/repo.git`) shows a **Clone** row.
Enter runs `git clone` into the displayed destination (the first watched folder by default);
**ctrl-enter** (**cmd-enter** on macOS) chooses another watched folder or a folder outside them.
If there is no watched
folder, it asks for one. The dialog stays open while it clones; if you close it, the clone
still finishes, and its success or failure is reported the next time the dialog opens after
it finishes. Private repositories work the way they do for `git` on your machine (SSH keys,
Git Credential Manager).

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
hotkey = "alt+space"
# switch_hotkey = "alt+q"  # window switcher; default alt+\, "" = off
scan_dirs = []          # folders to watch; manage with > Manage watched folders
# templates = ['C:\templates\rust-cli', "https://github.com/me/web-starter"]  # or > Manage templates
# forges = { "git.example.com" = "gitlab" }  # for pull request and CI links
scan_depth = 1          # >1 descends into non-git folders; default for every root
# [scan_rules.'C:\repos']   # optional override keyed by a path in scan_dirs
# depth = 3
# exclude = ["node_modules", "target", "tmp*"]
check_for_updates = true  # look for a new release about once a day
theme = "system"        # or "light" / "dark"; also set with > Theme
monitor = "cursor"      # the screen it opens on: "cursor", "focused" (the window in front) or "primary" (Windows only)
editor = "zed"          # set from the launcher; "" = file manager
editor_args = []        # e.g. ["--new-window"]
```

`scan_dirs` remains a list of paths. Optional `[scan_rules.'path']` entries override a
watched root: `depth` defaults to `scan_depth`, and `exclude` defaults to `[]`. Depth 1
lists immediate subfolders; higher depths descend into non-git folders and stop at git
repositories. An excluded name such as `target` matches a folder component exactly at
**any depth**; a trailing `*` such as `tmp*` matches folder names starting with `tmp`.
These are case-sensitive name/prefix rules, not path globs. Excluded directories are
neither listed nor traversed. Hidden directories are skipped as before, and manually added projects are
unaffected. The watched-folders page edits these rules without removing other config or
comments.

Picking an editor in the launcher updates only the `editor` line; the rest of the file,
comments included, is left alone.

Config and folder scans are re-read every time the dialog opens, so changes apply immediately.
That includes `hotkey`: a new shortcut starts working once the dialog has opened, by the old
shortcut or the tray icon. If none of the new shortcuts can be registered, the old ones keep
working and the dialog says why.
Manually added projects, hidden projects, names, added commands and open history live
in `projects.toml`.

If either file can't be read (a typo made editing it by hand), the dialog says where, e.g.
`config.toml, line 12: invalid type: string "2", expected u8`. Until it's fixed, a broken
`config.toml` means default settings, but for the editor when proj can still find it, and the
shortcuts stay as they were. A broken `projects.toml` is never written over: nothing is saved
until you fix or delete it.

## Windows installer

```powershell
.\scripts\build-installer.ps1              # cargo build --release + package
.\scripts\build-installer.ps1 -SkipBuild   # package the last release build
```

This produces `dist\proj-setup-<version>.exe` (workspace version from `Cargo.toml`). The script uses
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

1. Bump `[workspace.package].version` in `Cargo.toml` (e.g. `0.3.0`), run
   `cargo check --workspace` so `Cargo.lock` follows,
   commit and push to `main`.
2. Run the **Release** workflow: on GitHub, Actions → Release → Run workflow (branch `main`),
   or `gh workflow run release.yml`.

`.github/workflows/release.yml` refuses to run from any branch other than `main`, or when the
workspace version in `Cargo.toml` is already tagged. It then runs `scripts/build-installer.ps1` and
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
expose the accessory activation policy. The macOS editor and window icons and the window
switcher (through the Accessibility API) are written but haven't been run on a Mac yet. On
on Linux the switcher lists no windows and list rows have no icons.
