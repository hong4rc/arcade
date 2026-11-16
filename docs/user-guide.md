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

**Switching to a new build.** An old daemon keeps running the old code. Run `arcade upgrade` and the daemon continues as the new program without ending any session (see 5.13). Only a daemon older than that feature, or one with a different protocol (`arcade` then says "version mismatch"), needs `arcade kill-server` (this closes every session) before you run `arcade` again.

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
| `arcade upgrade [--force] [--restart] [<program>]` | Make the running daemon continue as a newer `arcade` (the program you ran, or the file you give) without ending any session or program (see 5.13) |
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

The status bar does not change when you press the prefix: the windows stay where they are, and the `PREFIX` word at the right end shows that the prefix is waiting. If you want a line of hints instead, set `[keys] hint = true` in the configuration file (then reload with the prefix and `r`). With it on, half a second after the prefix, when nothing else was pressed, the bar is replaced by one line of hints taken from the key table in force, for example `| - split  hjkl focus  HJKL size  c new  1-9 win  t tree  a notes  z zoom  [ copy  ? all keys`; a line that is too narrow is cut from the end but always keeps `? all keys`. Pressing a key brings the bar back and the action runs as usual, and while Ctrl is held (see below) the line shows at once and stays. Either way the panes are never drawn over; the full list of keys is always at the prefix and `?` (section 3.4). Rebinding a key changes the hint line; with `[status] position = "off"` there is no bar and so no hint. The line that appears while a pane swap waits for its direction (prefix, then `S`) is not a hint: it stays on whatever `hint` says, because the swap is waiting for that answer.

**Holding Ctrl to go on.** After the prefix, a key pressed on its own (`h`) runs its command once and the table closes: whatever you type next goes to the program in the pane, so a letter is never eaten. To repeat a command, keep **Ctrl** (the modifier of the prefix) held down after the prefix and press the keys with it: `Ctrl+h` moves focus left as `h` does, `Ctrl+Shift+h` is `H` (resize), and so on, as many times as you like and with no time limit, for any command in the table. With `[keys] hint = true`, the status bar shows the hint line while Ctrl is held. Let go of Ctrl and the next key goes to the pane as ordinary typing (it is not lost); `Esc` right after the prefix closes the table. A key that means nothing is passed over while you hold on. Pressing the prefix key itself still sends it to the program (so with the default prefix, `Ctrl+m` while holding Ctrl sends it on and closes the table). A prefix without a modifier (for example `f12`), and a terminal with old key codes that cannot tell Ctrl keys apart (`arcade config check --host legacy`), run one command for each prefix. Older files with `repeat-window-ms` or `repeat = true` still load; they do nothing, and `arcade config check` says so.

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
| `v` / `V` | Start selecting by character / by line. The same key again drops the selection; the other one keeps the starting point and changes the kind (`V` after `v` takes whole rows from the starting row to the cursor row, `v` after `V` goes back to characters) |
| `y` | Copy the selection and leave (`Enter` does nothing in copy mode, so it cannot copy by accident; bind it in `[keys.copy]` if you want it to) |
| `Y` | Copy the selection and stay in copy mode |
| `o` | Open the link under the cursor (see "Links" below); copy mode goes on |
| `s` | Jump: type what to go to, then the label of the match (see "Jump" below) |
| `[` / `]` | Move the cursor to the previous / next shell prompt (see "Commands and their output" below) |
| `O` | Select the output of the command the cursor is in; again, the output of the one before (see below) |
| `q`, `Enter` | Leave (nothing is copied; with a selection too) |
| `Esc` | Cancel (drop the selection, or leave if nothing is selected) |

**Where you are in the history.** A pane in copy mode (also when the wheel scrolled it) shows `[above/history]` at the right end of its first row, as tmux does: how many lines the view is above the bottom, over how many lines of history there are. At the bottom of copy mode it says `[0/<history>]`, so the mark does not come and go while you move; at the top of the history both numbers are the same. Each scrolled pane has its own mark. It is drawn over the view only: it is never selected, copied, searched, or counted by a jump, and a pane too narrow for it shows none. It is drawn again only when a number changes, and it costs nothing while a pane is at the bottom outside copy mode. `[scrollback] show-position = false` turns it off.

**Which state copy mode is in.** The word at the right end of the status bar says it: `COPY` while you only move, `VISUAL` while a selection by character is active, `V-LINE` while a selection by line is active, and `JUMP` followed by what you typed while a jump is open.

**Jump.** `s` opens a jump (as the flash plugin does in Neovim). Type the characters to go to: every match on the rows you see is highlighted, and each match gets a one-letter label drawn on the character right after it. Type a label to move the cursor to the start of that match; the jump ends. More typing narrows the matches. A label letter is never a character that could continue a match, so a key is either a label or more text to look for, never both. A lower case search matches both cases; an upper case letter makes it exact. With more matches than labels (26 letters), the matches nearest to the cursor get them and the others stay highlighted until you type more. `Enter` goes to the first match after the cursor (wrapping to the first), `Backspace` takes the last character back, `Esc` cancels and puts the cursor and the view back. If a selection is active (`v` or `V`), the jump moves the cursor and the selection stretches to it, with its starting point where it was; with no selection, the jump only moves. Example: to copy from here to the word `deploy` farther down, press `v`, `s`, type `deploy`, type the label shown after it, then `y`.

**Commands and their output.** If the shell tells Arcade where its prompts and the output of each command are (the shell prompt marks, OSC 133, section 5.12), copy mode knows the commands. `[` and `]` move the cursor to the previous and the next prompt. `O` selects, by line, the output of the command the cursor is in: from the first row after the prompt to the row before the next prompt, without the empty rows at the end; with the cursor on a prompt (for example on the last one, where copy mode starts), it selects the output of the command before it, and `O` again goes one command further back. Then `y` copies it. Outside copy mode, the prefix then `Y` copies the output of the last finished command of the pane straight to the clipboard and says how many lines were copied. In a pane with no prompt marks, or showing a full-screen program, these keys only show a message on the status bar. Arcade reads the marks only when you press one of these keys, so nothing is slowed down while output goes by.

**Links.** A program can mark text as a link (the hidden kind, OSC 8, for example what `ls --hyperlink` or many tools print): Arcade passes those links on to the terminal window, so Ghostty can open them (Cmd+click, or Ctrl+Shift+click while Arcade has the mouse). A link whose address holds a control character, a non-ASCII character or a space, or is longer than 2048 bytes, is not passed on and is drawn as plain text. A pane that is scrolled back, or in copy mode, keeps the links of the rows it shows (the same rules apply); the selection and search colours are drawn over a link without breaking it, and the position mark `[above/history]` covers the link under it. So that you can see what can be opened without hovering, the cells of a link that is passed on are drawn with a dotted underline (a cell the program underlined itself keeps the program's underline and its colour; a link that is not passed on is not underlined; the copied text is not changed). `[links] underline = false` turns that off.

The `o` key of copy mode opens the link the cursor is on. It takes first the hidden link of the cell under the cursor; otherwise an address written in the text (`http://`, `https://` or `file://`, in any case) that the cursor is on, also when the terminal broke it over two rows of the pane, and also when a program broke a long address by hand at the width of the pane and went on with it, indented, on the next row (over any number of rows, from any of them). That last join is a guess, so it is strict: the first row must end in an address whose last cell is at or within two cells of the right edge, and the next row must be indentation and then a word that is not a new address and has an address character in it (`/ . ? = & % + - _ #` or a digit); a plain word, a sentence, a list of addresses, a blank row or a table is never joined. The status bar shows the whole address that was opened. A sentence's punctuation at the end of the address (`. , ; : ! ? ' "`, a closing bracket without an opening one, `>`) is not part of it. Only those three kinds of address are opened; for anything else, or when there is no link under the cursor, the status bar says so and nothing else happens. The address is given to the opener as one single argument, never through a shell. The opener is `open` on macOS and `xdg-open` elsewhere, and can be changed with `[links] command` (a list of words, the program first; the address is added at the end), for example `command = ["firefox", "--new-tab"]`.

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

The key list (prefix, then `?`) shows one row for each action, in groups with a heading each: Panes, Windows, Sessions, Copy and paste, Notifications, Arcade. A row has the action in words, the keys that run it after the prefix (for example `h, left`), and the keys that run it with no prefix, written without the modifier that the column title names (`With ctrl+alt`: `h, left` there means `ctrl+alt+h` and `ctrl+alt+left`). A column with no key is empty, and an action with no key at all has no row (use the command list, prefix and `:`, for those). The window numbers are one row (`go to window N`, keys `1-9`), and `Enter` on it goes to window 1. The rows show what is bound now, so rebinding, unbinding, adding a key or the `nav` setting changes them. The choice moves from row to row and skips the headings. `Enter` runs the chosen action.

The key list filters by the action, by the command name (`pane.zoom`) and by any key of the row, written as in `arcade keys list` (`ctrl+m h`, `ctrl+alt+g`); a heading whose group has no match disappears. The keys of copy mode and of the lists are not shown until something is typed: they work only inside copy mode or a list, so `Enter` on such a row closes the list and does nothing; `arcade keys list` prints them all. A key that this terminal cannot send (section 5.11) has a `!` after it, and the key line at the bottom says so.

The key list uses most of the screen and keeps a margin. When the screen is wide enough and one column of groups would not fit, the list is drawn in two columns (a group may continue at the top of the second column, where its heading says `(continued)`), so that on a window of 120 columns by 36 rows the whole list fits; otherwise it is one column that scrolls, and the heading of the group of the chosen row stays on top. Too narrow for the columns, the rows are cut with `…`. When the box is too narrow for its key line, the hints that matter least are left out first (`type to filter`, then `up down move`, then `enter run`), so that `esc close` and what `!` means stay.

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

A pane line carries the mark of its own notification (`▲`, `◆`, `■` or `●`, section 5.8), and a window or session line carries the most urgent mark of the panes inside it, even when its branch is closed, so that you can see from the tree where something is waiting. The marks have the same colours as on the status bar.

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
| `[keys] hint` | `true` shows a line of key hints on the status bar half a second after the prefix and while Ctrl is held; `false` leaves the bar alone (section 3.1) | `false` |
| `[keys] repeat-window-ms` | Accepted so old files load; it does nothing (keys repeat while Ctrl is held, section 3.1) | none |
| `[keys.prefix]` (or `[keys.after-prefix]`) | Rebind the keys after the prefix | table in 3.1 |
| `[keys.direct]` | Rebind the direct layer keys (do not write the modifier, Arcade adds it) | table in 3.2 |
| `[keys.copy]` | Rebind the copy mode keys | table in 3.3 |
| `[keys.list]` | Rebind the selection list keys (a letter that is bound no longer goes into the filter box) | tables in 3.4 and 3.5 |
| `[status] position` | `"bottom"`, `"top"` or `"off"` | `"bottom"` |
| `[status] number` | Where the window number stands in a tab: `"left"` of the name, `"right"` of it, or `"off"` (the keys that use numbers still work). The number has a colour of its own, apart from the name | `"left"` |
| `[status] tab-format` | What a tab says besides the number, a text with the fields `{name}`, `{title}`, `{program}` and `{directory}` (section 5.3) | `"{name}"` |
| `[status] title-format` | The title Arcade sets on the outer terminal's window, with the fields `{session}`, `{number}`, `{name}`, `{title}`, `{program}` and `{directory}`; an empty text means Arcade sets none | `"{session}:{number} {name} · {title}"` |
| `[window] name-from` | Where the automatic name of a window comes from: `"directory"`, `"program"` or `"title"` (section 5.3) | `"directory"` |
| `[window] name-follow` | Whether the automatic name goes on following its source after the window was made | `false` |
| `[session] on-end` | What happens to your screen when the session you are in ends: `"switch"` moves you to another session (the one you used last, else the first in the list) and shows `session <old> ended; now in <new>` on the status bar for a few seconds; `"detach"` leaves Arcade as before. With no other session left, Arcade exits either way | `"switch"` |
| `[window] base-index` | The number of the first window | `1` |
| `[window] renumber` | Number the windows again when a window goes away | `true` |
| `[scrollback] lines` | How many lines of scrollback to keep for each pane | `50000` |
| `[scrollback] show-position` | Show `[above/history]` on the first row of a pane that is scrolled back or in copy mode (section 3.3) | `true` |
| `[clipboard] mode` | `"osc52"`, `"command"` or `"off"` | `"osc52"` |
| `[clipboard] command` | A command that receives the text on its standard input; required when `mode = "command"` | none |
| `[clipboard] allow-pane-write` | Let the program in a pane set the clipboard | `true` |
| `[links] underline` | Draw the cells of a hidden link that is passed on with a dotted underline, unless the program underlined them | `true` |
| `[links] command` | The program that opens a link with `o` in copy mode, as a list of words; the address is added as the last word | `["open"]` on macOS, `["xdg-open"]` elsewhere |
| `[upgrade] auto` | An `arcade` that attaches and finds the running daemon older than itself first asks the daemon to continue as this arcade (see 5.13); `false` only prints a hint | `true` |
| `[power] prevent-sleep` | Keep the machine from sleeping on its own while idle, as long as the daemon runs | `true` |
| `[notification] auto-jump` | Go to the pane that sent a notification out when the terminal window gets the focus back (section 5.8) | `true` |
| `[notification] tab-mark` | The mark of a notification on the tab of its window (section 5.8) | `true` |
| `[notification] pane-mark` | The mark at the top right of a pane out of focus, in a window of several panes | `true` |
| `[notification] status-count` | What the count at the right end of the status bar counts: `"other-sessions"`, `"all"` or `"off"` | `"other-sessions"` |
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
- **Window names.** A window made now is called after the last part of the directory its first pane started in (`arcade`; `~` for your home directory), and the name then stays, whatever the shell does. `[window] name-from` changes the source: `"program"` is the program in the foreground of the focused pane (`zsh`, `nvim`), `"title"` is the title that program set (OSC 0 or 2; the directory until it sets one). With `[window] name-follow = true` the name goes on following its source (looked at about once a second, while the screen is drawn anyway). The prefix then `,` gives a window a name of your own, which then stays; renaming it to an empty name gives it back to the rule above at once. A session is first called after the directory it started in (with `-2`, `-3` added when that name is taken); the prefix then `$` renames it.
- **What a tab says.** A tab is the window number and the text of `[status] tab-format`, by default just the window's name. `[status] number` puts the number left of the name (default), right of it, or hides it; the number is drawn in a colour of its own. The format may use `{name}`, `{title}` (what the program in the focused pane set, cut to 24 cells with a `…`, drawn fainter than the name), `{program}` and `{directory}`; for example `"{name} {title}"` shows both. The tree and the window list show the same text after the number. A format with a field that does not exist is a mistake that `arcade config check` and the reload report, and the default is used. A program that changes its title often (a spinner) only makes the status bar draw again, not the panes.
- **How a format fills in.** `{{` and `}}` write a brace. A field that is empty is left out together with the text just before it (the text that joins it to what comes before), so no separator is left hanging: `{session}:{number} {name} · {title}` with no title is `work:1 sh`. The text before the first field that shows is left out too, unless it starts the format, and the text after the last field stays only while that field shows. A `{title}` equal to the `{name}` counts as empty, so nothing is said twice.
- **The title of the outer terminal** is made from `[status] title-format`: by default `<session>:<number> <name> · <title>`, the Arcade part first and the program's own title (the focused pane's, OSC 0 or 2) after it, so the window of your terminal says which session and window it shows and what runs in it. Leave `{title}` out of the format to keep the program's title off the window; an empty format (`title-format = ""`) means Arcade sets no title on your terminal at all. A title is cut to 200 characters and loses control characters. It is sent when it changes and only then.
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
- Click on a pane to move focus there. A left press on a divider (a line between two panes) and a drag move that divider: the two sides it separates are resized as the resize keys do (the same smallest sizes; a pointer that goes further stops the divider at the limit), the divider ends under the pointer, and the programs in the panes are told their new sizes at the screen update rate, not once for each mouse move. In nested splits the divider under the pointer moves, not another one. A cell where one divider ends on another belongs to the one it ends on. The button coming up, anywhere on the screen, ends the drag. A press on a divider that is not followed by a drag does nothing. Arcade takes this even when a program beside the divider turned on mouse reporting (a divider is not part of a pane): nothing is sent to a pane and no selection starts. A zoomed pane has no dividers. Clicking on a divider does nothing else. A left click on a window's tab on the status bar shows that window, as the prefix and its number does (the previous-window key then comes back to the one you left); a click on the session name, on the blank between tabs, on the words at the right end, on a tab that is cut away because the terminal is narrow, or with another button, does nothing, and neither the click nor a drag that starts on the bar reaches a program in a pane. This works with the bar at the top or at the bottom, also while a program has turned on the mouse. While a prompt, a message or the key hint takes the place of the bar, and while the tree or a list is open, a click on that row does nothing.
- Scrolling up over a pane enters copy mode and scrolls 3 lines per notch; scrolling down to the bottom leaves copy mode by itself (only when the scroll opened that copy mode; a copy mode opened with a key stays). Scrolling acts on the pane under the pointer, which does not need focus.
- Dragging in a pane enters copy mode and selects text from where you pressed to where you let go. Letting go only keeps the selection and does not copy: press the copy key (`y`) to copy, or the leave key to drop it. The selection does not spill into a neighbouring pane.
- A program in a pane that has turned on mouse reporting (Neovim, btop, lazygit) gets the mouse actions directly, with coordinates counted inside the pane, and Arcade does not select text or scroll in its place. To have Arcade handle it anyway, hold **Alt** while clicking, dragging or scrolling.
- The outer terminal has a selection of its own, usually used by holding Shift while dragging. That selection does not know pane boundaries and is outside Arcade's reach, so it can sweep across both panes.
- A program that asks for the old xterm mouse reporting forms 1005 or 1015 is recognised. A program that asks for all motion without a button (mode 1003) only receives motion while a button is held.

**5.8 Notifications.** A program in a pane can ask for your attention with a bell (BEL), with `ESC ] 9 ; <message> ST` (OSC 9) or with `ESC ] 777 ; notify ; <title> ; <body> ST` (OSC 777). Each of these makes a notification of the class `info` for that pane, with the source `terminal`. A progress report (`ESC ] 9 ; 4 ; ...`) is not a notification (see the next paragraph), and the kitty notification protocol (OSC 99) is swallowed without making one.

**Progress reports.** A program can show a progress bar in the window of your terminal with `ESC ] 9 ; 4 ; <state> ; <value> ST` (state 0 removes it, 1 is a value from 0 to 100, 2 an error, 3 an unknown amount, 4 paused). Arcade passes the report of the focused pane of the window you are looking at on to your terminal, so a terminal that draws it (Ghostty shows a bar under the title) does so as it does without Arcade. When you move to another pane or window, the bar of the pane you moved to is shown if it has one, and otherwise the bar is removed; when the program ends, and when you detach, the bar is removed. A report that arrives again with the same state and value is passed on again, because programs repeat it to keep the bar. A report with a state or value out of range is ignored. Some terminals take a bar away by themselves after some seconds without a new report (Ghostty does, after about 15), so a pane that no longer repeats its report loses its bar there too.

The tab of a window also shows the report of its panes, after the name, in a field that is always four cells wide, so that the tabs to its right do not move while the number grows: `  9%`, ` 42%`, `100%` for a value, `x30%` for an error (` err` when it has no value), `p55%` for a pause and ` ...` for an unknown amount (an error or a pause at 100 shows 99). The report of the focused pane is shown; when that pane has none, the report of another pane of the window is. The mark goes away when the report is removed. An error is drawn in the colours of an `error` notification, the other marks like the number of the tab.
- A pane has at most one notification. Four classes, most urgent first, each with a mark of one cell and a colour: `▲` approval, `◆` input, `■` error, `●` info. The mark is the same one cell wherever it is drawn, and a window shows its most urgent mark. A program in a pane can only make `info`; the other three come from `arcade notify` (see 5.9). Each place can be turned off:
  - **The tab** of the window wears the mark after its name (`tab-mark`).
  - **The pane** wears it in the top right cell, only when it is out of focus, the window has several panes and the pane is at least 12 columns wide (`pane-mark`). It covers that one cell of what the program drew; with one pane the tab already says it, and the pane you are in needs no mark.
  - **The count** at the right end of the status bar counts what the tabs cannot show, the panes of the other sessions, as `●2`, and draws nothing when there are none (`status-count`: `"other-sessions"`, the default, `"all"` counts every pane that has a notification, `"off"` draws nothing). It counts panes, not signals.
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

**5.13 Upgrading without ending sessions.** After you install a new `arcade`, just detach and attach: an `arcade` that attaches (plain `arcade`, `attach` or `new`) and finds the running daemon older than itself asks it to continue as this program first, prints one line (`arcade: upgraded 0.1.5 to 0.1.6, sessions kept`, or why it could not) and attaches. It never goes the other way: an older arcade does not replace a newer daemon. A program that the daemon could not take over is not offered again until that program changes. `[upgrade] auto = false` turns this off (a one-line hint is printed instead). You can also type `arcade upgrade` yourself. The daemon writes down everything it holds (sessions, windows, layouts, the screen and history of every pane, titles, notifications), checks that the new program can read it, and then replaces its own program in the same process. The programs in your panes keep running, because their process and terminal are never closed. Every attached screen is told to start the new program and attach to the same session, by itself, in the same terminal; you type nothing. It prints both versions, for example `upgraded 0.1.4 -> 0.1.5: kept 3 sessions, 5 windows, 8 panes`.

- If the new program fails at any point before it says it is ready (an error, a crash, or taking too long), the old program starts again from the same snapshot, keeps everything, and `arcade upgrade` says `the upgrade was undone: <why>`. The old program is kept as a private copy next to the socket until the new one is ready, so an install that replaced or removed the file cannot get in the way.
- `arcade upgrade` changes nothing and says why when the new program is the same file and version (use `--force` to do it anyway), is not a regular program of yours or of the system, can be written by other users, or cannot read the snapshot. A newer snapshot is refused by an older program, so going back to an older `arcade` is not supported: use `arcade kill-server` for that.
- A daemon older than this feature (0.1.3 and 0.1.4) cannot do it, and it can tell a client almost nothing: only each session's name and its number of windows and panes. `arcade upgrade` then lists what it can see (the sessions, and the directory and foreground program of each pane as the system shows them) and what would end, and asks you to type `yes` (or takes `--restart`). It then stops the old daemon, starts the new one and makes each session again by name with the same number of windows, each with a shell; with exactly one session the shells go to the directories of the old panes in the order they were made, and directories not used are printed. Not kept: the programs running in the panes, history and screen text, split layout and sizes, window names, focus. Run it in a plain terminal window, not inside an Arcade pane (it refuses there, since the restart would end it). Attaching with a newer arcade only prints a hint naming this command, once for each program; it never restarts by itself.
- Known limits: copy mode ends (the screen is shown again as it was); cells below the text that a program cleared with a colour of their own come back with the default colour; copy mode finds prompt marks only from after the upgrade (marks in the history before it are not kept); a screen on a client of an older version that does not know the message prints `bad message from the server` and you attach again by hand.
- Restoring sessions after a reboot or a crash is not built; this only keeps them across a replacement of the program.

## 6. Not built yet

**5.12 Shell prompt marks (for copy mode's `[`, `]`, `O` and the prefix `Y`).** The keys that work on the output of a command need the shell to say where a prompt starts, where what you type starts, and where the output starts. Shells do it with the OSC 133 marks: `A` at the start of the prompt, `B` at the end of the prompt (where your typing starts), `C` when the command starts running (where its output starts) and, optionally, `D;<exit code>` when it ends. Arcade needs at least `A`, `B` and `C`: without `B` and `C` the text you type and the output cannot be told from the prompt. `D` is sent by the snippets below because other tools expect it, but Arcade does not use it. Arcade writes none of this itself and does not touch your dotfiles; put one of these in the startup file of your shell. They work with a prompt made by starship too (the `B` mark is added after the prompt is built, so keep the line that adds it last).

For zsh (`~/.zshrc`, after `eval "$(starship init zsh)"` if you use starship):

```zsh
_arcade_precmd() { print -n "\e]133;D;$?\a\e]133;A\a"; }
_arcade_preexec() { print -n "\e]133;C\a"; }
_arcade_prompt_end() { [[ $PS1 == *$'\e]133;B'* ]] || PS1+=$'%{\e]133;B\a%}'; }
precmd_functions=(_arcade_precmd $precmd_functions _arcade_prompt_end)
preexec_functions+=(_arcade_preexec)
```

For bash 4.4 or newer (`~/.bashrc`; the bash 3.2 that comes with macOS has no `PS0`, so install a newer one or use zsh):

```bash
_arcade_precmd() { printf '\e]133;D;%s\a\e]133;A\a' "$?"; }
PROMPT_COMMAND="_arcade_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
PS1="${PS1}\[\e]133;B\a\]"
PS0='\e]133;C\a'
```

Check it with `printf` or just try it: run a command, enter copy mode (prefix, then `[`) and press `O`: the output of the command is selected. Both snippets are run by Arcade's own tests (zsh and bash) against a pane, so they are known to give the marks Arcade reads.

Things that belong to Phase 1 or to the design but are not in the current build:

- Mouse: double click to select a word, triple click to select a line, an option to copy on release, scrolling by itself when dragging past a pane edge.
- For after Phase 1: several clients on one session (a second client entering replaces the first), saving and restoring sessions after the daemon stops, rectangle selection, changing layouts by mouse, themes.
