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
| `arcade config check [--host kitty\|legacy] [<file>]` | Check a configuration file and list its errors, then the bindings that cannot work on the terminal (see 5.11), without a daemon |
| `arcade keys list [--host kitty\|legacy] [<file>]` | Print every key binding in force, table by table, with where it comes from (`default` or `config`), then the bindings that cannot work. Reads the file only, so it needs no daemon and gives the same text every time |
| `arcade keys test [<file>]` | Show, for each key you press, the name Arcade receives, its bytes and what it does in each table. `Ctrl+C` quits and the terminal is given back as it was |

**When typed inside an Arcade pane** (Arcade sets the variables `ARCADE` and `ARCADE_PANE`): `arcade new` and `arcade attach` do not nest an Arcade inside Arcade. They switch the screen you are looking at to the other session. `arcade new -d` still only creates a session and changes nothing. Plain `arcade` (and any other way that would attach) does nothing there: it prints `already inside Arcade` and exits with code 1, because attaching would draw Arcade inside the pane it runs in and throw out the client that is already there. The daemon refuses such an attach too. An Arcade of another socket (`--socket` or `ARCADE_SOCKET` set to a different path) is real nesting and still attaches.

## 3. Default keys

There are two ways to give a command. The **prefix** is pressing `Ctrl+m` and letting go, then pressing one key. The **direct layer** is holding `Ctrl+Alt` together with a key, without the prefix.

The four default movement keys are `h` (left), `j` (down), `k` (up), `l` (right), and the four arrow keys always work alongside them. Change them with the `nav` key (section 4). In the tables below, "movement key" means these keys.

### 3.1 After the prefix (Ctrl+m)

If half a second after the prefix nothing else is pressed, the status bar is replaced by one line of hints taken from the key table in force, for example `| - split  hjkl focus  HJKL size  c new  1-9 win  t tree  a notes  z zoom  [ copy  ? all keys`. A line that is too narrow is cut from the end but always keeps `? all keys`, which opens the full list. Pressing a key brings the bar back and the action runs as usual. The panes are never drawn over. While Ctrl is held (see below) the line shows at once and stays. Rebinding a key changes the line; with `[status] position = "off"` there is no bar and so no hint.

**Holding Ctrl to go on.** After the prefix, a key pressed on its own (`h`) runs its command once and the table closes: whatever you type next goes to the program in the pane, so a letter is never eaten. To repeat a command, keep **Ctrl** (the modifier of the prefix) held down after the prefix and press the keys with it: `Ctrl+h` moves focus left as `h` does, `Ctrl+Shift+h` is `H` (resize), and so on, as many times as you like and with no time limit, for any command in the table. While Ctrl is held the status bar shows the hint line. Let go of Ctrl and the next key goes to the pane as ordinary typing (it is not lost); `Esc` right after the prefix closes the table. A key that means nothing is passed over while you hold on. Pressing the prefix key itself still sends it to the program (so with the default prefix, `Ctrl+m` while holding Ctrl sends it on and closes the table). A prefix without a modifier (for example `f12`), and a terminal with old key codes that cannot tell Ctrl keys apart (`arcade config check --host legacy`), run one command for each prefix. Older files with `repeat-window-ms` or `repeat = true` still load; they do nothing, and `arcade config check` says so.

| Key | Action |
|---|---|
| movement key | Move focus to the pane in that direction |
| Shift + movement key (for example `H`) | Resize the pane in that direction |
| `S` then a movement key (letter or arrow) | Swap the current pane with its neighbour in that direction; focus follows the pane. While `S` waits for the direction the status bar says so; `Esc` cancels. With no neighbour, a message shows on the status bar. Holding Ctrl: `Ctrl+Shift+s`, then `Ctrl+h`, and again |
| `\|` | Split the pane to the right |
| `-` | Split the pane downwards |
| `;` | Go back to the previous pane |
| `z` | Zoom or unzoom the pane |
| `x` | Close the pane |
| `Space` | Next layout, in a cycle: panes side by side with equal widths, stacked with equal heights, one main pane on the left (the first pane, 60 percent of the width) with the others stacked on the right |
| `=` | Make the panes equal in size (the way of splitting stays; panes along the same axis differ by at most one cell) |
| `!` | Break the current pane out into a window of its own, right after this window; with only one pane, a message shows on the status bar |
| `c` | New window |
| `1` to `9` | Select the window by number |
| `Tab` | Go back to the previous window |
| `u` | Next window |
| `y` | Previous window |
| `<` | Move the window to the left |
| `>` | Move the window to the right |
| `,` | Rename the window |
| `$` | Rename the session |
| `s` | List of sessions |
| `w` | List of the windows of this session; type to filter by name, `Enter` to go there |
| `t` | Tree of sessions, windows and panes: look and jump (section 3.5) |
| `m` | Lift the current pane to put it down somewhere else (section 3.5). `m` used to move a window to another session; the command `window.move-to-session` is still in the command palette (`:`) but has no default key |
| `M` | Lift the current window to put it down somewhere else (section 3.5) |
| `a` | List of the notifications (section 5.8) |
| `A` | Clear every notification |
| `q` | Clear the notification of the pane you are in |
| `[` | Enter copy mode |
| `]` | Paste the internal buffer into the pane |
| `:` | Command palette: every named command, each line with the keys bound to it; type to filter, `Enter` to run (a command that needs a number or a name asks on the prompt line) |
| `?` | List of the keys that exist |
| `d` | Detach from the session |
| `g` | Lock or unlock Arcade (see 5.1) |
| `r` | Reload the configuration file |
| `R` | Redraw the whole screen |

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
| `Ctrl+Alt` + `a` | Go to the pane whose notification is the most urgent; press again for the next one (section 5.8) |

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

The box keeps one size and one place for as long as it is open. Its width comes from the widest line any state could show (every line in full, the key line, the title) and its height from the whole list, so typing in the filter box only blanks rows and never moves or resizes the box. A line wider than the box, and the lines below the last row, are cut with an ellipsis (`…`). On a small terminal the box shrinks to fit the screen. The last line of every list says which keys work right now, built from your key table: if you rebind a key in `[keys.list]`, the line shows the new key, and a key you unbind disappears from it.

### 3.5 Tree of sessions, windows and panes: jumping, lifting and putting down

The prefix then `t` opens a tree with three tiers: sessions, the windows inside them, the panes inside those. A window line shows the window's name as the status bar does. A pane line shows the title the program set (or the name of the program the pane started if there is none), then `in <directory>` with the last part of the directory the pane stands in, and `running <program>` when a program other than the first one is in the foreground, for example `zsh in arcade running nvim`. The filter looks at all of that. The place you are in is marked (and the session and window carry a `*`); when the tree opens, the branch that holds that place is open and the other branches are closed. The tree does not update by itself over time; if something disappears while the tree is open, the tree is rebuilt at the next drawing and the choice moves to the nearest item that still exists.

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

The box has the same fixed size and place as the other lists (section 3.4). It has two lines at the bottom. The first says in words what `Enter` will do (for example `Enter: put pane beside 2 server`), or why `Enter` does nothing (for example `Enter does nothing: nothing matches`). The second is the key line, and it follows what the tree is doing: looking (`up down move  left right close/open  enter go  space lift  type to filter  esc close`), moving something (`up down choose where  enter put down  esc cancel the lift`), or filtering (`up down move  enter go  ctrl+u clear filter  esc close`, without folding and lifting). While something is lifted the title says what, for example `Tree · moving pane zsh`.

A pane line carries the mark of its own notification (`[!]`, `[?]`, `[x]` or `[i]`, section 5.8), and a window or session line carries the most urgent mark of the panes inside it, even when its branch is closed, so that you can see from the tree where something is waiting. The marks have the same colours as on the status bar.

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
| `[keys] repeat-window-ms` | Accepted so old files load; it does nothing (keys repeat while Ctrl is held, section 3.1) | none |
| `[keys.prefix]` (or `[keys.after-prefix]`) | Rebind the keys after the prefix | table in 3.1 |
| `[keys.direct]` | Rebind the direct layer keys (do not write the modifier, Arcade adds it) | table in 3.2 |
| `[keys.copy]` | Rebind the copy mode keys | table in 3.3 |
| `[keys.list]` | Rebind the selection list keys (a letter that is bound no longer goes into the filter box) | tables in 3.4 and 3.5 |
| `[status] position` | `"bottom"`, `"top"` or `"off"` | `"bottom"` |
| `[session] on-end` | What happens to your screen when the session you are in ends: `"switch"` moves you to another session (the one you used last, else the first in the list) and shows `session <old> ended; now in <new>` on the status bar for a few seconds; `"detach"` leaves Arcade as before. With no other session left, Arcade exits either way | `"switch"` |
| `[window] base-index` | The number of the first window | `1` |
| `[window] renumber` | Number the windows again when a window goes away | `true` |
| `[scrollback] lines` | How many lines of scrollback to keep for each pane | `50000` |
| `[clipboard] mode` | `"osc52"`, `"command"` or `"off"` | `"osc52"` |
| `[clipboard] command` | A command that receives the text on its standard input; required when `mode = "command"` | none |
| `[clipboard] allow-pane-write` | Let the program in a pane set the clipboard | `true` |
| `[power] prevent-sleep` | Keep the machine from sleeping on its own while idle, as long as the daemon runs | `true` |
| `[notification] auto-jump` | Go to the pane that sent a notification out when the terminal window gets the focus back (section 5.8) | `true` |
| `[notification] push-when-host-focused` | Send a notification out even when the terminal window has the focus, for a pane other than the one in focus | `true` |
| `[notification] osc` | The escape sequence a notification from `arcade notify` goes out as: `"777"` (title and body) or `"9"` (one message) | `"777"` |
| `[notification] push-interval` | Seconds between two sends out for one pane (0 for no limit) | `5` |
| `[notification] bell-interval` | Seconds between two bells for one terminal (0 for no limit) | `2` |
| `[notification] mute-sources` | List of source names whose signals are dropped, for example `["claude-code"]` | none |
| `[notification.approval]`, `.input`, `.error`, `.info` | Four true or false keys per class: `indicator` (makes a mark), `bell` (rings the terminal), `osc` (asks the terminal for a notification of its own), `clear-on-focus` (looking at the pane takes it away) | `approval`, `input`, `error`: indicator, bell and osc on, clear-on-focus off; `info`: indicator, osc and clear-on-focus on, bell off |

How to bind a key in the `[keys.*]` tables:

```toml
[keys.prefix]
"q" = "pane.zoom"                                   # bind a command
"x" = false                                         # remove a default key
"H" = "pane.resize-left"                            # bind a shifted key
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

**5.3 Window names and the window title.**
- **Window names.** A window is called after the program in the foreground of its focused pane (`zsh`, `nvim`, `claude`), and the name follows the program (it is looked at about once a second, while the screen is drawn anyway). The prefix then `,` gives a window a name of your own, which then stays; renaming it to an empty name gives it back to the program. A session is first called after the directory it started in (with `-2`, `-3` added when that name is taken); the prefix then `$` renames it.
- **The title of the outer terminal** is `<title> · <session>:<number>`. The title is the one the program in the focused pane set (OSC 0 or 2), or the window's name when it has set none; when it is the same word as the session's name, the title is just `<session>:<number>`. A title is cut to 200 characters and loses control characters.
- **A title lasts as long as its program.** When the program that set a title has ended, the title goes and the window's name shows again (a title set by the shell itself stays). A program that saves the title before it changes it (`CSI 22 t`) and restores it when it ends (`CSI 23 t`) works as in other terminals: up to 10 titles are kept for each pane, and the icon-only forms (`22;1`, `23;1`) change nothing. One limit: a program that sets a title and ends faster than Arcade can read what it wrote looks like the shell's own, and its title stays.

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

**5.8 Notifications.** A program in a pane can ask for your attention with a bell (BEL), with `ESC ] 9 ; <message> ST` (OSC 9) or with `ESC ] 777 ; notify ; <title> ; <body> ST` (OSC 777). Each of these makes a notification of the class `info` for that pane, with the source `terminal`. A progress report (`ESC ] 9 ; 4 ; ...`) is not a notification and is ignored, and the kitty notification protocol (OSC 99) is swallowed without making one.
- A pane has at most one notification. Its mark shows on the tab of its window, at the top right of the pane (when the pane is at least 12 columns wide), and in a counter at the right end of the status bar that counts panes, not signals. Four classes, most urgent first, each with a mark of its own and a colour: `[!]` approval, `[?]` input, `[x]` error, `[i]` info. A window shows its most urgent mark. A program in a pane can only make `info`; the other three come from `arcade notify` (see 5.9).
- An `info` notification goes away as soon as you look at the pane (it takes the focus), and is not kept at all for the pane you are looking at. A notification of another class stays until you type into the pane.
- A notification goes away with its pane, and stays when you detach and attach again.
- A pane that sends more than 20 signals in a second keeps only the last of the excess.
- **Going to a pane that needs you.** `Ctrl+Alt+a` goes to the pane whose notification is the most urgent; among the same class, the pane that has waited longest comes first. The pane you are in is skipped, so pressing the key again goes to the next one; across sessions, the client changes session. When nothing waits, or only the pane you are in has a notification, a message says so.
- **The list.** The prefix then `a` opens a list of the notifications, most urgent first, each line with the class, the source, the title and message, how long it has waited and where the pane is (as `work › 1 arcade › zsh in arcade`). Type to filter, Up and Down (or `Ctrl+p` and `Ctrl+n`) to move, `Enter` to go to the pane, `Delete` to clear the chosen notification (the list closes when none is left), `Esc` to close.
- **Clearing by hand.** The prefix then `q` clears the notification of the pane you are in; the prefix then `A` clears all of them. The command names are `notification.next`, `notification.list`, `notification.clear` and `notification.clear-all`; `arcade run notification.list` prints the notifications as lines of tab-separated fields (class, source, pane, seconds waited, place, title, message).
- **Out to the terminal you run Arcade in.** When a notification takes effect for a pane that you are not looking at, Arcade asks the terminal (Ghostty, iTerm2 and others) to get your attention: a notification of its own (the escape sequence OSC 777, or OSC 9 for a message that came without a title), and, for the classes `approval`, `input` and `error`, a bell. Whether a sound or a system notification shows is the terminal's decision. Arcade builds a new sequence from the cleaned title and message; it never passes on what the pane wrote. A signal from `arcade notify` goes out as OSC 777 with the title, or the source when there is no title, and the message as body; a bare bell from a program makes the indicator only.
- **When Arcade thinks you are looking.** Arcade turns on the focus reporting of the terminal (mode 1004) and learns when the terminal window gets or loses the focus. You are looking at a pane when it has the focus in Arcade and the terminal window has the focus. Nothing goes out for that pane, and an `info` notification for it is not kept. When the terminal window has lost the focus, even the pane in focus counts as not looked at: its notification is kept and sent out, and an `info` one goes away when the window gets the focus back. A terminal that does not report focus is taken as always focused after you attach, so notifications for the pane in focus stay quiet there.
- **Going back to the pane that called.** When a notification was sent out while the terminal window did not have the focus, Arcade goes to the pane of the newest one that still stands the next time the window gets the focus (the status bar shows nothing for this; the screen simply changes). Arcade cannot tell a click on the system notification from coming back to the window for another reason, so `Ctrl+Alt+o` (the key that goes back to the previous pane) returns to the pane you were in, as long as you have not moved since. `auto-jump = false` in `[notification]` turns the jump off.
- **Settings.** Which classes make a mark, ring, ask for a notification or go away when looked at, the two intervals, the muted sources and the jump are in the `[notification]` tables (section 4).
- **Limits.** A pane sends out at most once in 5 seconds (a signal that comes sooner is still shown as an indicator). A terminal is rung at most once in 2 seconds; bells that come sooner wait and ring once together when the 2 seconds end.
- A tab list that is longer than the screen is cut, so that the counters and the PREFIX, LOCK and COPY words at the right stay in view.
- When a pane already has a notification, a new one from the same source replaces it; one from another source replaces it only when its class is at least as urgent, and is dropped otherwise.
- Good to know: an `ESC ] 777 ; notify ; ; <body> ST` with an empty title counts as a message without a title (the same as OSC 9).
- Good to know: when the terminal window does not have the focus, the pane in focus keeps its notification and sends it out like any other pane (the usual case is an agent finishing while you are in another application); the notification of class `info` goes away when the window gets the focus back.
- Good to know: a terminal that does not report focus is taken as always focused after you attach, so it never sends out a notification for the pane in focus, and `info` for that pane is not kept.
- Good to know: the place column of the list of notifications reads like the tree path when filtered: session, window number and name, pane.

**5.9 `arcade notify`.** For hooks and scripts of other programs: it tells Arcade that a pane needs you.
- `arcade notify [--class info|input|approval|error] [--title <text>] [--message <text>] [--target <pane>] [--source <name>] [--strict]`. The class is `info` when not given. The pane is the one named by `--target`, or else the one the command runs in (the `ARCADE_PANE` variable that every pane has).
- `arcade notify clear [--target <pane> | --all]` takes one notification, or all of them, away.
- `arcade notify check [--target <pane>]` is for you to run by hand: it says which pane and which socket would be used and where the pane came from, tells whether the daemon answers, sends a test `info` notification, and exits with a non-zero code when any step fails.
- `arcade notify log` lists what became of the last 100 signals (how long ago, from where, class, source, pane, and what happened). It keeps no title and no message.
- It is made not to disturb the program that calls it: it waits at most 200 milliseconds, never retries, never starts a daemon, never prints, and exits with 0 whatever goes wrong. With `--strict` it prints the reason on the error output and exits with 1. Control characters in the title and message are removed, and the title, message and source are cut to 64, 256 and 32 characters.

**5.10 Example: telling Arcade when Claude Code needs you.** Arcade has no code of its own for any coding agent; this is only an example of the two ways to use what is above.
- *Nothing to set up.* Set the notification channel of Claude Code to a kind that writes an OSC escape sequence (for example the `ghostty` kind, which writes OSC 777). Arcade reads it from the pane, shows an `info` mark, and sends a notification out to your terminal as in 5.8.
- *With classes.* Add hooks to the Claude Code settings that call `arcade notify`. The names of the hooks and the values of `notification_type` change from one Claude Code version to the next, so check them against its current hooks documentation (these were seen on version 2.1.314, except the last one, which was not tried):

| Hook | Command |
|---|---|
| `Notification` with `notification_type` `permission_prompt` | `arcade notify --class approval --source claude-code` |
| `Notification` with `notification_type` `agent_needs_input`, `elicitation_dialog` or `elicitation_url_dialog` | `arcade notify --class input --source claude-code` |
| `Stop` | `arcade notify --class info --source claude-code` |
| `StopFailure` | `arcade notify --class error --source claude-code` |
| `UserPromptSubmit` | `arcade notify clear` |

If you use both ways, the notification from the hook replaces the one from the escape sequence that comes at the same time (same source or a class at least as urgent wins, see 5.8). To tell "finished a reply" from "finished the whole task", give the hooks different `--title` values; Arcade only knows the four classes. Keep prompts, commands and transcripts out of `--message`: Arcade keeps the class, title, message, source and time of each notification in memory until it is cleared.
- Run `arcade notify check` inside a pane to see which pane and socket a hook would use, and `arcade notify log` to see what became of the latest signals.

**5.11 Keys that cannot work, and what Arcade receives.** A terminal without the kitty keyboard protocol sends old key codes, and some keys cannot be told apart in them: `Ctrl` with a digit or a symbol, `Ctrl+m` and Enter, `Ctrl+i` and Tab, `Ctrl+Shift` and `Ctrl` with a letter, and anything with Cmd. Arcade asks the terminal when you attach. If it has only old key codes and a binding of your tables needs one of these keys, the status bar shows one line, for example `3 key bindings cannot work here (first: ctrl+m); run arcade keys list`, and again each time you reload the configuration. `arcade config check` and `arcade keys list` name every such binding with the reason, for example `warning: [keys.direct] ctrl+alt+1: the terminal does not send Ctrl with a digit`; run in a terminal they ask it themselves, and `--host legacy` or `--host kitty` says what to assume when there is no terminal to ask (without either, they say the host is not known and do not guess). Two more warnings do not depend on the terminal: a `direct` binding on the prefix key is never reached, and a `prefix` binding on the prefix key stops "prefix twice" from sending it to the program; on macOS, `Ctrl+Space` never reaches the terminal. The warnings never stop Arcade from starting.

To see what really arrives, run `arcade keys test` in the terminal you use. A key that shows nothing was taken by the system, the window manager or the terminal before it reached Arcade (for example `Alt+h` under AeroSpace), while `Ctrl+Alt+h` shows its name and the command it runs.

**5.12 Ending the session you are in.** Closing its last pane, `session.close` and `arcade kill-session` on it all follow `[session] on-end` (section 4). Sessions that already have a client of their own are not taken over; the move goes to a session nobody is looking at. Notifications of the ended session are dropped from the counter.

## 6. Not built yet

Things that belong to Phase 1 or to the design but are not in the current build:

- Mouse: double click to select a word, triple click to select a line, an option to copy on release, scrolling by itself when dragging past a pane edge, dragging a border to resize, clicking a tab on the status bar.
- For after Phase 1: several clients on one session (a second client entering replaces the first), saving and restoring sessions after the daemon stops, rectangle selection, changing layouts by mouse, themes.
