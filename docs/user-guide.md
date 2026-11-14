# Arcade user guide

This guide describes what the current build does. Every key was checked against `crates/daemon/src/keymap.rs` and every configuration key against `crates/daemon/src/config.rs`. Section 6 lists what is not built yet.

## 1. Building and running

The first time on a new machine:

```
./scripts/fetch-zig.sh
cargo build --release
```

The first command downloads Zig 0.17 into `.tools/zig` inside the repository (it installs nothing on the machine). The second produces `target/release/arcade`. Copy that file into a directory on your `PATH` if you want to type `arcade` anywhere.

To run, type `arcade`. If no daemon is running yet, the command starts one in the background.

**Switching to a new build.** An old daemon keeps running the old code. If the protocol differs, `arcade` says "version mismatch". Then run `arcade kill-server` (this closes every session) and run `arcade` again.

Default paths:

| Thing | Path |
|---|---|
| Socket | `$XDG_RUNTIME_DIR/arcade/default.sock`; without that variable, `$TMPDIR/arcade-<uid>/default.sock` (without `TMPDIR`, `/tmp/arcade-<uid>/default.sock`) |
| Configuration file | `~/.config/arcade/config.toml`, or the path in the `ARCADE_CONFIG` variable |

The option `--socket <path>`, placed first on the command line, uses another socket, which means another daemon, fully separate from the default one.

## 2. Commands in the terminal

| Command | What it does |
|---|---|
| `arcade` | Enter the most recently used session; create one if there is none |
| `arcade new [-d] [-s <name>] [<command>...]` | Create a session and enter it. `-d`: create without entering. `<command>`: the program that runs in the first pane (the shell by default) |
| `arcade attach [-t <session>]` | Enter a session |
| `arcade ls` | List the sessions |
| `arcade kill-session [-t <session>]` | Close a session |
| `arcade kill-server` | Stop the daemon and every session |
| `arcade run <command> [<argument>...]` | Run a named command (for example `session.detach`), meant for scripts |
| `arcade config check [<file>]` | Check a configuration file and list its errors, without a daemon |

**When typed inside an Arcade pane** (Arcade sets the variables `ARCADE` and `ARCADE_PANE`): `arcade new` and `arcade attach` do not nest an Arcade inside Arcade. They switch the screen you are looking at to the other session. `arcade new -d` still only creates a session and changes nothing.

## 3. Default keys

There are two ways to give a command. The **prefix** is pressing `Ctrl+m` and letting go, then pressing one key. The **direct layer** is holding `Ctrl+Alt` together with a key, without the prefix.

The four default movement keys are `h` (left), `j` (down), `k` (up), `l` (right), and the four arrow keys always work alongside them. Change them with the `nav` key (section 4). In the tables below, "movement key" means these keys.

### 3.1 After the prefix (Ctrl+m)

If half a second after the prefix nothing else is pressed, a small panel appears that lists the keys of this table (keys with the same action share one line, for example `h/left`). Pressing a key closes the panel and the action runs as usual. There is no way to turn this panel off yet.

"Repeat" means that after the key, the prefix key table stays open for `repeat-window-ms` (600 ms by default), so the next key can be pressed at once without the prefix.

| Key | Action | Repeat |
|---|---|---|
| movement key | Move focus to the pane in that direction | yes |
| Shift + movement key (for example `H`) | Resize the pane in that direction | yes |
| Ctrl + movement key (letter or arrow) | Swap the current pane with its neighbour in that direction; focus follows the pane. With no neighbour, a message shows on the status bar | yes |
| `\|` | Split the pane to the right | |
| `-` | Split the pane downwards | |
| `;` | Go back to the previous pane | |
| `z` | Zoom or unzoom the pane | |
| `x` | Close the pane | |
| `Space` | Next layout, in a cycle: panes side by side with equal widths, stacked with equal heights, one main pane on the left (the first pane, 60 percent of the width) with the others stacked on the right | yes |
| `=` | Make the panes equal in size (the way of splitting stays; panes along the same axis differ by at most one cell) | |
| `!` | Break the current pane out into a window of its own, right after this window; with only one pane, a message shows on the status bar | |
| `c` | New window | |
| `1` to `9` | Select the window by number | |
| `Tab` | Go back to the previous window | |
| `u` | Next window | yes |
| `y` | Previous window | yes |
| `<` | Move the window to the left | yes |
| `>` | Move the window to the right | yes |
| `,` | Rename the window | |
| `$` | Rename the session | |
| `s` | List of sessions | |
| `w` | List of the windows of this session; type to filter by name, `Enter` to go there | |
| `t` | Tree of sessions, windows and panes: look and jump (section 3.5) | |
| `m` | Lift the current pane to put it down somewhere else (section 3.5). `m` used to move a window to another session; the command `window.move-to-session` is still in the command palette (`:`) but has no default key | |
| `M` | Lift the current window to put it down somewhere else (section 3.5) | |
| `[` | Enter copy mode | |
| `]` | Paste the internal buffer into the pane | |
| `:` | Command palette: every named command, each line with the keys bound to it; type to filter, `Enter` to run (a command that needs a number or a name asks on the prompt line) | |
| `?` | List of the keys that exist | |
| `d` | Detach from the session | |
| `g` | Lock or unlock Arcade (see 5.1) | |
| `r` | Reload the configuration file | |
| `R` | Redraw the whole screen | |

### 3.2 Direct layer (Ctrl+Alt)

| Key | Action |
|---|---|
| `Ctrl+Alt` + movement key | Move focus to the pane in that direction |
| `Ctrl+Alt+Shift` + movement key | Resize the pane |
| `Ctrl+Alt` + `1` to `9` | Select the window by number |
| `Ctrl+Alt` + `,` | Previous window |
| `Ctrl+Alt` + `.` | Next window |
| `Ctrl+Alt` + `o` | Go back to the previous pane |
| `Ctrl+Alt` + `g` | Lock or unlock Arcade |

These keys do not repeat: hold the modifier and press again.

### 3.3 Copy mode (enter with the prefix, then `[`)

| Key | Action |
|---|---|
| movement key | Left, down, up, right |
| `w` / `b` / `e` | Next word / previous word / end of word |
| `0` / `^` / `$` | Start of line / first non-blank character / end of line |
| `g` / `G` | Start / end of all the content |
| `Ctrl+u` / `Ctrl+d` | Half a page up / down |
| `Ctrl+b` / `Ctrl+f`, `PageUp` / `PageDown` | A whole page up / down |
| `/` / `?` | Search forward / backward |
| `n` / `N` | Next / previous search result |
| `v` / `V` | Start selecting by character / by line |
| `y` or `Enter` | Copy the selection and leave |
| `Y` | Copy the selection and stay in copy mode |
| `q` | Leave |
| `Esc` | Cancel (drop the selection, or leave if nothing is selected) |

If your movement keys take over one of the keys above, the displaced key moves: `e` becomes `j`, `n` becomes `k`, `N` becomes `K`. A key with no place to move to has no key (see section 4).

### 3.4 Selection lists (sessions, windows, key list, command palette)

Every list has a filter box on its first line: type letters and only the lines that contain them remain (case does not matter), updating with each key. Because letters go into the filter box, lists do not use letter keys for movement (`h j k l`, or your `nav`).

| Key | Action |
|---|---|
| Up arrow or `Ctrl+p` | Up one line |
| Down arrow or `Ctrl+n` | Down one line |
| Letters, digits, punctuation | Typed into the filter box |
| `Backspace` | Delete one character of the filter box |
| `Ctrl+u` | Clear the whole filter box |
| `Enter` | Choose the chosen line (the first line of the filtered result) |
| `Esc` | Close |

The key list (prefix, then `?`) filters by key and by command name, for example type `zoom`.

### 3.5 Tree of sessions, windows and panes: jumping, lifting and putting down

The prefix then `t` opens a tree with three tiers: sessions, the windows inside them, the panes inside those. A pane line shows the title the program set, or the name of the running program if there is none. The place you are in is marked (and the session and window carry a `*`); when the tree opens, the branch that holds that place is open and the other branches are closed. The tree does not update by itself over time; if something disappears while the tree is open, the tree is rebuilt at the next drawing and the choice moves to the nearest item that still exists.

| Key | Action |
|---|---|
| Up arrow or `Ctrl+p` | The previous item of the same tier; past the items of one parent, the last item of the previous parent (that branch opens by itself) |
| Down arrow or `Ctrl+n` | The same in the other direction |
| Right arrow | Go in: from a session to its current window, from a window to its focused pane |
| Left arrow | Step out to the parent item |
| `Enter` | Nothing lifted: jump to the chosen item (change session, select window, focus pane) and close the tree. Something lifted: put it down |
| `Space` | Lift the chosen item (a pane or a window); pressing again on that item puts it down. On a session it only shows a one-line notice |
| `Esc` | Something lifted: put it down, the tree stays open. Nothing lifted: close the tree, and the screen returns exactly as it was |
| Letters, digits, punctuation | Go into the filter box; the tree turns flat, each matching line with its path, for example `work › 2 server › zsh` |

The bottom line of the box always says in words what `Enter` will do, or why `Enter` does nothing.

`m` opens the tree with the current pane already lifted (the choice on its window); `M` opens the tree with the current window already lifted (the choice on its session). Putting down works like this:

| Lifted | Put on | Result |
|---|---|---|
| Pane | Another pane | Splits the place of the target pane: the target stays on the left, the moved pane goes on the right |
| Pane | A window | The same as putting it on the focused pane of that window |
| Pane | A session | Becomes a new window at the end of that session |
| Window | A window | Stands right after the target window (the same session reorders, another session moves it over) |
| Window | A session | Becomes the last window of that session |

The program in the pane is not restarted and its scrollback stays; only its size changes. An invalid target (the lifted item itself, the window of that very pane, a pane inside the lifted window, a window already at the end of its session, a window with no room to split) changes nothing. A zoomed target window is unzoomed first. When the last pane leaves a window, that window closes; when the last window leaves a session, that session closes, and you follow the item that was moved. After putting down, the tree closes and focus follows the moved item. The left, right and lift keys can be rebound in `[keys.list]` (commands `list.left`, `list.right`, `list.lift`); a key that is bound no longer goes into the filter box.

## 4. Configuration file

TOML format. A misspelled key is reported with a suggestion ("did you mean ..."). Check before use with `arcade config check`. Reload while running: prefix, then `r`. With no file, every default is used.

| Key | Meaning | Default |
|---|---|---|
| `[keys] prefix` | The prefix key | `"ctrl+m"` |
| `[keys] direct-modifier` | The modifier of the direct layer; `"none"` turns this layer off | `"ctrl+alt"` |
| `[keys] nav` | The four movement characters, in the order left, down, up, right | `"hjkl"` |
| `[keys] repeat-window-ms` | How long the prefix panel stays open after a repeating key, in milliseconds | `600` |
| `[keys.prefix]` (or `[keys.after-prefix]`) | Rebind the keys after the prefix | table in 3.1 |
| `[keys.direct]` | Rebind the direct layer keys (do not write the modifier, Arcade adds it) | table in 3.2 |
| `[keys.copy]` | Rebind the copy mode keys | table in 3.3 |
| `[keys.list]` | Rebind the selection list keys (a letter that is bound no longer goes into the filter box) | tables in 3.4 and 3.5 |
| `[status] position` | `"bottom"`, `"top"` or `"off"` | `"bottom"` |
| `[window] base-index` | The number of the first window | `1` |
| `[window] renumber` | Number the windows again when a window goes away | `true` |
| `[scrollback] lines` | How many lines of scrollback to keep for each pane | `50000` |
| `[clipboard] mode` | `"osc52"`, `"command"` or `"off"` | `"osc52"` |
| `[clipboard] command` | A command that receives the text on its standard input; required when `mode = "command"` | none |
| `[clipboard] allow-pane-write` | Let the program in a pane set the clipboard | `true` |
| `[power] prevent-sleep` | Keep the machine from sleeping on its own while idle, as long as the daemon runs | `true` |

How to bind a key in the `[keys.*]` tables:

```toml
[keys.prefix]
"q" = "pane.zoom"                                   # bind a command
"x" = false                                         # remove a default key
"H" = { run = "pane.resize-left", repeat = true }   # bind and let it repeat
"w" = "window.select 3"                             # a command with an argument
```

Binding one key twice in the same table is an error. See the command names with the prefix, then `?`.

### Example file for the author's machine

Prefix `ctrl+m`, direct layer `ctrl+alt`, movement `hnei` (left `h`, down `n`, up `e`, right `i`), status bar on top:

```toml
[keys]
prefix = "ctrl+m"
direct-modifier = "ctrl+alt"
nav = "hnei"

[status]
position = "top"
```

With `nav = "hnei"`, copy mode changes as follows: `h n e i` move left, down, up, right; search next is `k` (in place of `n`), search previous is `K` (in place of `N`), end of word is `j` (in place of `e`). Selection lists use no letters for movement (see 3.4).

## 5. Good to know

**5.1 Lock.** The prefix then `g` (or `Ctrl+Alt+g`) puts Arcade in the locked state: every key goes straight to the program in the pane, and only the lock key unlocks. Use it when running Arcade inside Arcade or inside tmux.

**5.2 The `Ctrl+m` prefix needs the newer keyboard protocol.** On an old-style terminal, `Ctrl+m` is identical to the Enter key, so Arcade can tell them apart only when the outer terminal supports the kitty keyboard protocol (Ghostty does). When Arcade runs inside tmux, `Ctrl+m` cannot be used, because tmux does not pass this protocol to the program inside. To use another prefix, set `prefix = "ctrl+b"` (or any key you like) in `[keys]`.

**5.3 Window title.** The title that the program in a pane sets (OSC 0 or 2) is passed on to the title of the outer terminal. The window name is set with the prefix then `,`.

**5.4 Clipboard.** By default `y` in copy mode sends the text to the outer terminal with OSC 52, so the outer terminal must allow it. The `command` mode pushes the text into a command you choose, for example:

```toml
[clipboard]
mode = "command"
command = "pbcopy"
```

A program in a pane can also set the clipboard with OSC 52 when `allow-pane-write = true`; set it to `false` to block that. Reading the clipboard from a pane is always refused. Each copy shows a notice on the status bar that goes away by itself after 3 seconds. The internal buffer keeps the latest copy; the prefix then `]` pastes it.

**5.5 Using it next to tmux.** The two programs are independent, each with its own daemon and socket. Arcade does not touch tmux.

**5.6 Keeping the machine awake.** As long as the daemon runs, Arcade keeps the machine from sleeping on its own while idle (on macOS with a `caffeinate` process that is a child of the daemon, on Linux with `systemd-inhibit` if present). The display still turns off by the machine's settings, and closing the lid or pressing Sleep still puts the machine to sleep. On battery, leaving the daemon alive overnight drains the battery faster; to turn this off, set `prevent-sleep = false` in `[power]` and reload the configuration.

**5.7 Mouse.** Arcade turns on mouse reporting in the outer terminal when you enter a session and turns it off when you leave.
- Click on a pane to move focus there. Clicking on a divider or on the status bar does nothing.
- Scrolling up over a pane enters copy mode and scrolls 3 lines per notch; scrolling down to the bottom leaves copy mode by itself (only when the scroll opened that copy mode; a copy mode opened with a key stays). Scrolling acts on the pane under the pointer, which does not need focus.
- Dragging in a pane enters copy mode and selects text from where you pressed to where you let go. Letting go only keeps the selection and does not copy: press the copy key (`y`) to copy, or the leave key to drop it. The selection does not spill into a neighbouring pane.
- A program in a pane that has turned on mouse reporting (Neovim, btop, lazygit) gets the mouse actions directly, with coordinates counted inside the pane, and Arcade does not select text or scroll in its place. To have Arcade handle it anyway, hold **Alt** while clicking, dragging or scrolling.
- The outer terminal has a selection of its own, usually used by holding Shift while dragging. That selection does not know pane boundaries and is outside Arcade's reach, so it can sweep across both panes.
- A program that asks for the old xterm mouse reporting forms 1005 or 1015 is recognised. A program that asks for all motion without a button (mode 1003) only receives motion while a button is held.

## 6. Not built yet

Things that belong to Phase 1 or to the design but are not in the current build:

- Notification (an indicator, a counter, `arcade notify`, BEL and OSC passed to the outer terminal).
- Mouse: double click to select a word, triple click to select a line, an option to copy on release, scrolling by itself when dragging past a pane edge, dragging a border to resize, clicking a tab on the status bar.
- `arcade keys list` and `arcade keys test` on the command line (inside Arcade, use the prefix then `?`).
- A warning for bindings that cannot work (beyond the errors of `config check`).
- For after Phase 1: several clients on one session (a second client entering replaces the first), saving and restoring sessions after the daemon stops, rectangle selection, changing layouts by mouse, themes.
- Copy mode has one known limit: two identical lines next to each other can be off by one line while the content is scrolling.
