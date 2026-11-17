//! The configuration file.
//!
//! One TOML file with a table for each part of Arcade. Reading it never stops the daemon: a
//! file with a mistake is reported with the line of each mistake, and what was running before
//! stays. Nothing is read from the real file in tests; the path is always given.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use arcade_term::{Key, Mods};
use toml::Spanned;
use toml::de::{DeTable, DeValue};

use crate::commands;
use crate::format::{Field, Template};
use crate::keymap::{Binding, Keymap, KeysConfig, Table, normalize, parse_key};

pub const DEFAULT_TAB_FORMAT: &str = "{name}";
pub const DEFAULT_TITLE_FORMAT: &str = "{session}:{number} {name} \u{b7} {title}";
pub const TAB_FIELDS: &[Field] = &[Field::Name, Field::Title, Field::Program, Field::Directory];
pub const TITLE_FIELDS: &[Field] = &[
    Field::Session,
    Field::Number,
    Field::Name,
    Field::Title,
    Field::Program,
    Field::Directory,
];

/// Where the status bar sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusPosition {
    Bottom,
    Top,
    Off,
}

/// Where the number of a window stands in its tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberPosition {
    Left,
    Right,
    Off,
}

/// How the tabs and the session label of the status bar are drawn: with round ends (the
/// powerline half circles, which need a font that has them) or as plain blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStyle {
    Pill,
    Flat,
}

/// Where the automatic name of a window comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameFrom {
    /// The last part of the directory of the window's focused pane.
    Directory,
    /// The program in the foreground of the focused pane.
    Program,
    /// The title the program in the focused pane set.
    Title,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardMode {
    /// Ask the host terminal to set its clipboard (OSC 52).
    Osc52,
    /// Pipe the text to a command.
    Command,
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardConfig {
    pub mode: ClipboardMode,
    pub command: Option<String>,
    /// Whether a program in a pane may set the clipboard.
    pub allow_pane_write: bool,
}

/// How a link is opened from copy mode, and how links are drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinksConfig {
    /// Cells of a hidden link that is passed on are underlined (a dotted underline), unless the
    /// program underlined them itself.
    pub underline: bool,
    /// The program and its first arguments; the address is added as the last argument. `None`
    /// is `open` on macOS and `xdg-open` elsewhere.
    pub command: Option<Vec<String>>,
}

impl Default for LinksConfig {
    fn default() -> Self {
        LinksConfig {
            underline: true,
            command: None,
        }
    }
}

/// The four key tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableName {
    Direct,
    Prefix,
    Copy,
    List,
}

impl TableName {
    pub fn word(self) -> &'static str {
        match self {
            TableName::Direct => "direct",
            TableName::Prefix => "prefix",
            TableName::Copy => "copy",
            TableName::List => "list",
        }
    }
}

/// One line of a `[keys.<table>]` table: a binding set, or a default one removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyOverride {
    pub table: TableName,
    pub key: Key,
    pub binding: Option<Binding>,
}

/// What a notification of one class does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassPolicy {
    /// It makes a mark on the tab, the pane and the counter.
    pub indicator: bool,
    /// It rings the host terminal.
    pub bell: bool,
    /// It asks the host terminal for a notification of its own.
    pub osc: bool,
    /// Looking at the pane takes it away.
    pub clear_on_focus: bool,
}

/// The kind of escape sequence a notification from `arcade notify` goes out as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OscKind {
    /// A title and a body.
    Osc777,
    /// One message.
    Osc9,
}

/// The `[notification]` settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationConfig {
    /// In the order of `Class::BY_URGENCY`: approval, input, error, info.
    pub classes: [ClassPolicy; 4],
    /// Send a signal out even when the host terminal has the focus, when the pane that gave it
    /// does not.
    pub push_when_host_focused: bool,
    pub osc: OscKind,
    /// How seldom a pane sends a signal out, and how seldom a client is rung.
    pub push_interval: Duration,
    pub bell_interval: Duration,
    /// Signals from these sources are dropped.
    pub muted_sources: Vec<String>,
    /// Go to the pane that sent a signal out when the host terminal gets the focus back.
    pub auto_jump: bool,
    /// The mark on the tab of a window that has a notification.
    pub tab_mark: bool,
    /// The mark at the top right of a pane out of focus, in a window of several panes.
    pub pane_mark: bool,
    /// The count at the right end of the status bar.
    pub status_count: StatusCount,
}

/// What the count at the right end of the status bar counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCount {
    Off,
    /// Only the notifications the tabs cannot show: those of the other sessions.
    OtherSessions,
    /// Every pane that has one.
    All,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        let waits = ClassPolicy {
            indicator: true,
            bell: true,
            osc: true,
            clear_on_focus: false,
        };
        NotificationConfig {
            classes: [
                waits,
                waits,
                waits,
                ClassPolicy {
                    bell: false,
                    clear_on_focus: true,
                    ..waits
                },
            ],
            push_when_host_focused: true,
            osc: OscKind::Osc777,
            push_interval: Duration::from_secs(5),
            bell_interval: Duration::from_secs(2),
            muted_sources: Vec::new(),
            auto_jump: true,
            tab_mark: true,
            pane_mark: true,
            status_count: StatusCount::OtherSessions,
        }
    }
}

impl NotificationConfig {
    pub fn class(&self, class: crate::notify::Class) -> ClassPolicy {
        let at = crate::notify::Class::BY_URGENCY
            .iter()
            .position(|c| *c == class)
            .unwrap_or(3);
        self.classes[at]
    }
}

/// What happens to a client whose session ends while other sessions exist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnSessionEnd {
    /// The client goes to another session.
    Switch,
    /// The client leaves, with the message that the session ended.
    Detach,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub keys: KeysConfig,
    pub overrides: Vec<KeyOverride>,
    pub status_position: StatusPosition,
    /// Where a window's number stands in its tab, or that it is not shown.
    pub tab_number: NumberPosition,
    pub tab_style: TabStyle,
    /// How long a command (by the marks a shell prompt leaves, OSC 133) runs before the tab says
    /// so, and how long it runs for its end to be left on the tab of a pane that is out of sight;
    /// zero turns each off.
    pub command_running_after: Duration,
    pub command_finished_after: Duration,
    /// What a tab says besides the number (fields: name, title, program, directory).
    pub tab_format: Template,
    /// The title Arcade sets on the host terminal's window (fields: session, number, name,
    /// title, program, directory); empty means Arcade sets none.
    pub title_format: Template,
    /// Where the automatic name of a window comes from, and whether it goes on following its
    /// source after the window was made.
    pub window_name_from: NameFrom,
    pub window_name_follow: bool,
    /// The number of the first window.
    pub base_index: usize,
    /// Whether the windows' numbers close up when one goes.
    pub renumber: bool,
    pub scrollback_lines: Option<usize>,
    /// Whether a pane in copy mode shows `[above/history]` on its first row.
    pub scroll_position: bool,
    pub clipboard: ClipboardConfig,
    pub links: LinksConfig,
    /// Whether the daemon keeps the machine from sleeping because it sat idle.
    pub prevent_sleep: bool,
    /// Whether an `arcade` that attaches and finds the daemon older than itself asks it to
    /// continue as that arcade first (`[upgrade] auto`).
    pub upgrade_auto: bool,
    pub on_session_end: OnSessionEnd,
    pub notification: NotificationConfig,
    /// The file sets `repeat-window-ms` or a binding with `repeat = true`: both are accepted
    /// and do nothing, because keys repeat only while the prefix's modifier is held.
    pub timed_repeat_ignored: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            keys: KeysConfig::default(),
            overrides: Vec::new(),
            status_position: StatusPosition::Bottom,
            tab_number: NumberPosition::Right,
            command_running_after: Duration::from_secs(2),
            command_finished_after: Duration::from_secs(10),
            tab_style: TabStyle::Pill,
            tab_format: Template::parse(DEFAULT_TAB_FORMAT, TAB_FIELDS).expect("a valid default"),
            title_format: Template::parse(DEFAULT_TITLE_FORMAT, TITLE_FIELDS)
                .expect("a valid default"),
            window_name_from: NameFrom::Directory,
            window_name_follow: false,
            base_index: 1,
            renumber: true,
            scrollback_lines: None,
            scroll_position: true,
            prevent_sleep: true,
            upgrade_auto: true,
            on_session_end: OnSessionEnd::Switch,
            notification: NotificationConfig::default(),
            timed_repeat_ignored: false,
            links: LinksConfig::default(),
            clipboard: ClipboardConfig {
                mode: ClipboardMode::Osc52,
                command: None,
                allow_pane_write: true,
            },
        }
    }
}

impl Config {
    /// The key tables this file makes: the defaults for its settings, changed by its bindings.
    pub fn keymap(&self) -> Keymap {
        let mut map = Keymap::defaults(&self.keys);
        for o in &self.overrides {
            let table: &mut Table = match o.table {
                TableName::Direct => &mut map.direct,
                TableName::Prefix => &mut map.prefix,
                TableName::Copy => &mut map.copy,
                TableName::List => &mut map.list,
            };
            match &o.binding {
                Some(b) => {
                    table.insert(o.key, b.clone());
                }
                None => {
                    table.remove(&o.key);
                }
            }
        }
        map
    }
}

/// A mistake in the file, with the line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// The file's usual place, or the one the environment names.
pub fn default_path(env_config: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = env_config.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    home.filter(|h| !h.is_empty())
        .map(|h| Path::new(h).join(".config/arcade/config.toml"))
}

/// Read the file at `path`. A missing file is the defaults; a file that cannot be read or has
/// a mistake is an error that says what and where.
pub fn load(path: &Path) -> Result<Config, Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text)
            .map_err(|errors| errors.iter().map(ToString::to_string).collect::<Vec<_>>()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(vec![format!("cannot read {}: {e}", path.display())]),
    }
}

struct Reader<'a> {
    text: &'a str,
    errors: Vec<ConfigError>,
}

impl Reader<'_> {
    fn line_of(&self, offset: usize) -> usize {
        1 + self.text.as_bytes()[..offset.min(self.text.len())]
            .iter()
            .filter(|b| **b == b'\n')
            .count()
    }

    fn error(&mut self, offset: usize, message: impl Into<String>) {
        let line = self.line_of(offset);
        self.errors.push(ConfigError {
            line,
            message: message.into(),
        });
    }

    fn string<'v>(&mut self, key: &str, value: &'v Spanned<DeValue<'_>>) -> Option<&'v str> {
        match value.get_ref() {
            DeValue::String(s) => Some(s.as_ref()),
            _ => {
                self.error(value.span().start, format!("{key} must be a string"));
                None
            }
        }
    }

    fn boolean(&mut self, key: &str, value: &Spanned<DeValue<'_>>) -> Option<bool> {
        match value.get_ref() {
            DeValue::Boolean(b) => Some(*b),
            _ => {
                self.error(value.span().start, format!("{key} must be true or false"));
                None
            }
        }
    }

    fn integer(&mut self, key: &str, value: &Spanned<DeValue<'_>>) -> Option<usize> {
        let parsed = match value.get_ref() {
            DeValue::Integer(i) => i.as_str().replace('_', "").parse::<usize>().ok(),
            _ => None,
        };
        if parsed.is_none() {
            self.error(
                value.span().start,
                format!("{key} must be a whole number, zero or more"),
            );
        }
        parsed
    }

    fn table<'v, 'i>(
        &mut self,
        key: &str,
        value: &'v Spanned<DeValue<'i>>,
    ) -> Option<&'v DeTable<'i>> {
        match value.get_ref() {
            DeValue::Table(t) => Some(t),
            _ => {
                self.error(value.span().start, format!("{key} must be a table"));
                None
            }
        }
    }
}

/// The modifiers written as `alt`, `ctrl+alt`, or `none`.
fn parse_modifier(text: &str) -> Result<Option<Mods>, String> {
    if text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let mut mods = Mods::NONE;
    for word in text.split('+') {
        match word.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "option" | "meta" => mods.alt = true,
            "shift" => mods.shift = true,
            "super" | "cmd" | "command" => mods.super_key = true,
            _ => return Err(format!("unknown modifier \"{word}\"")),
        }
    }
    Ok(Some(mods))
}

fn merge(key: Key, mods: Mods) -> Key {
    normalize(Key {
        code: key.code,
        mods: Mods {
            shift: key.mods.shift || mods.shift,
            alt: key.mods.alt || mods.alt,
            ctrl: key.mods.ctrl || mods.ctrl,
            super_key: key.mods.super_key || mods.super_key,
        },
    })
}

/// A command written as `name arg ...`.
fn parse_command(text: &str) -> Result<(String, Vec<String>), String> {
    let mut words = text.split_whitespace();
    let name = words.next().ok_or_else(|| "an empty command".to_string())?;
    let args: Vec<String> = words.map(str::to_string).collect();
    if !commands::is_known(name) {
        return Err(format!("unknown command \"{name}\"{}", suggestion(name)));
    }
    commands::parse(name, &args)?;
    Ok((name.to_string(), args))
}

/// ", did you mean ..." for a command name that is a near miss.
fn suggestion(name: &str) -> String {
    let near = commands::all_names()
        .filter(|candidate| {
            let a = name.split('.').next_back().unwrap_or(name);
            let b = candidate.split('.').next_back().unwrap_or(candidate);
            a == b || candidate.starts_with(name) || name.starts_with(candidate)
        })
        .take(3)
        .collect::<Vec<_>>();
    if near.is_empty() {
        String::new()
    } else {
        format!(" (did you mean {}?)", near.join(" or "))
    }
}

/// How many single-character edits turn one word into the other.
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (diagonal + usize::from(ca != *cb))
                .min(row[j] + 1)
                .min(above + 1);
            diagonal = above;
        }
    }
    row[b.len()]
}

/// The message for a key that is not known: `unknown key "section.key"`, and the closest known
/// key when one is at most two edits away.
fn unknown_key(section: &str, key: &str, known: &[&str]) -> String {
    let full = |k: &str| {
        if section.is_empty() {
            k.to_string()
        } else {
            format!("{section}.{k}")
        }
    };
    let kind = if section.is_empty() { "section" } else { "key" };
    let mut message = format!("unknown {kind} \"{}\"", full(key));
    let closest = known
        .iter()
        .map(|k| (edit_distance(key, k), *k))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d);
    if let Some((_, k)) = closest {
        message.push_str(&format!(", did you mean \"{}\"?", full(k)));
    }
    message
}

fn read_notification(r: &mut Reader<'_>, table: &DeTable<'_>, config: &mut NotificationConfig) {
    const KEYS: &[&str] = &[
        "auto-jump",
        "push-when-host-focused",
        "osc",
        "push-interval",
        "bell-interval",
        "mute-sources",
        "tab-mark",
        "pane-mark",
        "status-count",
        "approval",
        "input",
        "error",
        "info",
    ];
    for (n, v) in table.iter() {
        let name = n.get_ref().as_ref();
        match name {
            "auto-jump" => {
                if let Some(b) = r.boolean("auto-jump", v) {
                    config.auto_jump = b;
                }
            }
            "tab-mark" => {
                if let Some(b) = r.boolean("tab-mark", v) {
                    config.tab_mark = b;
                }
            }
            "pane-mark" => {
                if let Some(b) = r.boolean("pane-mark", v) {
                    config.pane_mark = b;
                }
            }
            "status-count" => match r.string("status-count", v) {
                Some("other-sessions") => config.status_count = StatusCount::OtherSessions,
                Some("all") => config.status_count = StatusCount::All,
                Some("off") => config.status_count = StatusCount::Off,
                Some(other) => r.error(
                    v.span().start,
                    format!(
                        "status-count is \"other-sessions\", \"all\" or \"off\", not \"{other}\""
                    ),
                ),
                None => {}
            },
            "push-when-host-focused" => {
                if let Some(b) = r.boolean("push-when-host-focused", v) {
                    config.push_when_host_focused = b;
                }
            }
            "osc" => match r.string("osc", v) {
                Some("777") => config.osc = OscKind::Osc777,
                Some("9") => config.osc = OscKind::Osc9,
                Some(other) => r.error(
                    v.span().start,
                    format!("osc is \"777\" or \"9\", not \"{other}\""),
                ),
                None => {}
            },
            "push-interval" => {
                if let Some(s) = r.integer("push-interval", v) {
                    config.push_interval = Duration::from_secs(s as u64);
                }
            }
            "bell-interval" => {
                if let Some(s) = r.integer("bell-interval", v) {
                    config.bell_interval = Duration::from_secs(s as u64);
                }
            }
            "mute-sources" => match v.get_ref() {
                DeValue::Array(items) => {
                    config.muted_sources = items
                        .iter()
                        .filter_map(|item| r.string("mute-sources", item).map(str::to_string))
                        .collect();
                }
                _ => r.error(
                    v.span().start,
                    "mute-sources must be a list of source names",
                ),
            },
            "approval" | "input" | "error" | "info" => {
                let Some(class) = crate::notify::Class::parse(name) else {
                    continue;
                };
                let Some(inner) = r.table(&format!("notification.{name}"), v) else {
                    continue;
                };
                let at = crate::notify::Class::BY_URGENCY
                    .iter()
                    .position(|c| *c == class)
                    .unwrap_or(3);
                for (k, value) in inner.iter() {
                    let slot = match k.get_ref().as_ref() {
                        "indicator" => &mut config.classes[at].indicator,
                        "bell" => &mut config.classes[at].bell,
                        "osc" => &mut config.classes[at].osc,
                        "clear-on-focus" => &mut config.classes[at].clear_on_focus,
                        other => {
                            r.error(
                                k.span().start,
                                unknown_key(
                                    &format!("notification.{name}"),
                                    other,
                                    &["indicator", "bell", "osc", "clear-on-focus"],
                                ),
                            );
                            continue;
                        }
                    };
                    if let Some(b) =
                        r.boolean(&format!("notification.{name}.{}", k.get_ref()), value)
                    {
                        *slot = b;
                    }
                }
            }
            other => r.error(n.span().start, unknown_key("notification", other, KEYS)),
        }
    }
}

fn nav_chars(text: &str) -> Result<[char; 4], String> {
    let chars: Vec<char> = text.chars().collect();
    let Ok(four) = <[char; 4]>::try_from(chars) else {
        return Err("nav needs exactly four characters (left, down, up, right)".to_string());
    };
    for (i, c) in four.iter().enumerate() {
        if c.is_whitespace() || c.is_control() || four[..i].contains(c) {
            return Err("nav needs four different, visible characters".to_string());
        }
    }
    Ok(four)
}

fn read_keys(r: &mut Reader<'_>, table: &DeTable<'_>, config: &mut Config) {
    // The settings come first, since the bindings of the direct table depend on them.
    for (name, value) in table.iter() {
        let key = name.get_ref().as_ref();
        match key {
            "prefix" if matches!(value.get_ref(), DeValue::Table(_)) => {}
            "prefix" => {
                if let Some(text) = r.string("prefix", value) {
                    match parse_key(text) {
                        Ok(k) => config.keys.prefix = k,
                        Err(e) => r.error(value.span().start, e),
                    }
                }
            }
            "direct-modifier" => {
                if let Some(text) = r.string("direct-modifier", value) {
                    match parse_modifier(text) {
                        Ok(m) => config.keys.direct_modifier = m,
                        Err(e) => r.error(value.span().start, e),
                    }
                }
            }
            "nav" => {
                if let Some(text) = r.string("nav", value) {
                    match nav_chars(text) {
                        Ok(n) => config.keys.nav = n,
                        Err(e) => r.error(value.span().start, e),
                    }
                }
            }
            "hint" => {
                if let Some(on) = r.boolean("hint", value) {
                    config.keys.hint = on;
                }
            }
            "repeat-window-ms" => {
                if let Some(ms) = r.integer("repeat-window-ms", value) {
                    config.keys.repeat_window = Duration::from_millis(ms as u64);
                    config.timed_repeat_ignored = true;
                }
            }
            "direct" | "after-prefix" | "copy" | "list" => {}
            _ => r.error(
                name.span().start,
                unknown_key(
                    "keys",
                    key,
                    &[
                        "prefix",
                        "direct-modifier",
                        "nav",
                        "hint",
                        "repeat-window-ms",
                        "direct",
                        "after-prefix",
                        "copy",
                        "list",
                    ],
                ),
            ),
        }
    }
    // `prefix` is a setting (a string) and may also be the table of bindings (a table), but a
    // file cannot have both under one name, so `after-prefix` is the same table under another.
    let mut seen: HashMap<(TableName, Key), usize> = HashMap::new();
    for (name, value) in table.iter() {
        let word = name.get_ref().as_ref();
        let which = match (word, value.get_ref()) {
            ("direct", _) => TableName::Direct,
            ("prefix", DeValue::Table(_)) | ("after-prefix", _) => TableName::Prefix,
            ("copy", _) => TableName::Copy,
            ("list", _) => TableName::List,
            _ => continue,
        };
        let Some(bindings) = r.table(&format!("keys.{word}"), value) else {
            continue;
        };
        for (key_text, bound) in bindings.iter() {
            read_binding(r, which, key_text, bound, config, &mut seen);
        }
    }
}

fn read_binding(
    r: &mut Reader<'_>,
    table: TableName,
    key_text: &Spanned<std::borrow::Cow<'_, str>>,
    bound: &Spanned<DeValue<'_>>,
    config: &mut Config,
    seen: &mut HashMap<(TableName, Key), usize>,
) {
    let at = key_text.span().start;
    let key = match parse_key(key_text.get_ref().as_ref()) {
        Ok(k) => k,
        Err(e) => {
            r.error(at, e);
            return;
        }
    };
    let key = if table == TableName::Direct {
        match config.keys.direct_modifier {
            Some(m) => merge(key, m),
            None => {
                r.error(at, "[keys.direct] needs a direct-modifier, and it is none");
                return;
            }
        }
    } else {
        key
    };
    let line = r.line_of(at);
    if let Some(first) = seen.insert((table, key), line) {
        r.error(
            at,
            format!(
                "the key {} is bound twice in [keys.{}] (lines {first} and {line})",
                crate::keymap::key_name(key),
                table.word()
            ),
        );
        return;
    }
    let binding = match bound.get_ref() {
        DeValue::Boolean(false) => None,
        DeValue::Boolean(true) => {
            r.error(
                bound.span().start,
                "use false to remove a binding, or a command",
            );
            return;
        }
        DeValue::String(text) => match parse_command(text) {
            Ok((command, args)) => Some(Binding {
                command,
                args,
                repeat: false,
            }),
            Err(e) => {
                r.error(bound.span().start, e);
                return;
            }
        },
        DeValue::Table(t) => {
            let mut run = None;
            let mut repeat = false;
            for (n, v) in t.iter() {
                match n.get_ref().as_ref() {
                    "run" => run = r.string("run", v).map(str::to_string),
                    "repeat" => repeat = r.boolean("repeat", v).unwrap_or(false),
                    other => r.error(n.span().start, format!("unknown key \"{other}\"")),
                }
            }
            let Some(text) = run else {
                r.error(
                    bound.span().start,
                    "a binding table needs run = \"command\"",
                );
                return;
            };
            match parse_command(&text) {
                Ok((command, args)) => {
                    config.timed_repeat_ignored |= repeat;
                    Some(Binding {
                        command,
                        args,
                        repeat,
                    })
                }
                Err(e) => {
                    r.error(bound.span().start, e);
                    return;
                }
            }
        }
        _ => {
            r.error(
                bound.span().start,
                "a binding is a command, false, or { run = \"command\", repeat = true }",
            );
            return;
        }
    };
    config.overrides.push(KeyOverride {
        table,
        key,
        binding,
    });
}

/// Read the text of a configuration file.
pub fn parse(text: &str) -> Result<Config, Vec<ConfigError>> {
    let mut r = Reader {
        text,
        errors: Vec::new(),
    };
    let root = match DeTable::parse(text) {
        Ok(root) => root.into_inner(),
        Err(e) => {
            let at = e.span().map_or(0, |s| s.start);
            r.error(at, e.message().to_string());
            return Err(r.errors);
        }
    };
    let mut config = Config::default();
    for (name, value) in root.iter() {
        let section = name.get_ref().as_ref();
        let Some(table) = r.table(section, value) else {
            continue;
        };
        match section {
            "keys" => read_keys(&mut r, table, &mut config),
            "status" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "position" => {
                            config.status_position = match r.string("position", v) {
                                Some("bottom") => StatusPosition::Bottom,
                                Some("top") => StatusPosition::Top,
                                Some("off") => StatusPosition::Off,
                                Some(other) => {
                                    r.error(
                                        v.span().start,
                                        format!("position is bottom, top or off, not \"{other}\""),
                                    );
                                    config.status_position
                                }
                                None => config.status_position,
                            };
                        }
                        "number" => match r.string("number", v) {
                            Some("left") => config.tab_number = NumberPosition::Left,
                            Some("right") => config.tab_number = NumberPosition::Right,
                            Some("off") => config.tab_number = NumberPosition::Off,
                            Some(other) => r.error(
                                v.span().start,
                                format!("number is left, right or off, not \"{other}\""),
                            ),
                            None => {}
                        },
                        "tab-style" => match r.string("tab-style", v) {
                            Some("pill") => config.tab_style = TabStyle::Pill,
                            Some("flat") => config.tab_style = TabStyle::Flat,
                            Some(other) => r.error(
                                v.span().start,
                                format!("tab-style is pill or flat, not \"{other}\""),
                            ),
                            None => {}
                        },
                        "running-after" => {
                            if let Some(s) = r.integer("running-after", v) {
                                config.command_running_after = Duration::from_secs(s as u64);
                            }
                        }
                        "finished-after" => {
                            if let Some(s) = r.integer("finished-after", v) {
                                config.command_finished_after = Duration::from_secs(s as u64);
                            }
                        }
                        "tab-format" => {
                            if let Some(text) = r.string("tab-format", v) {
                                match Template::parse(text, TAB_FIELDS) {
                                    Ok(t) => config.tab_format = t,
                                    Err(e) => r.error(v.span().start, format!("tab-format: {e}")),
                                }
                            }
                        }
                        "title-format" => {
                            if let Some(text) = r.string("title-format", v) {
                                match Template::parse(text, TITLE_FIELDS) {
                                    Ok(t) => config.title_format = t,
                                    Err(e) => r.error(v.span().start, format!("title-format: {e}")),
                                }
                            }
                        }
                        other => r.error(
                            n.span().start,
                            unknown_key(
                                "status",
                                other,
                                &[
                                    "position",
                                    "number",
                                    "tab-style",
                                    "tab-format",
                                    "title-format",
                                    "running-after",
                                    "finished-after",
                                ],
                            ),
                        ),
                    }
                }
            }
            "window" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "base-index" => {
                            if let Some(i) = r.integer("base-index", v) {
                                config.base_index = i;
                            }
                        }
                        "renumber" => {
                            if let Some(b) = r.boolean("renumber", v) {
                                config.renumber = b;
                            }
                        }
                        "name-from" => match r.string("name-from", v) {
                            Some("directory") => config.window_name_from = NameFrom::Directory,
                            Some("program") => config.window_name_from = NameFrom::Program,
                            Some("title") => config.window_name_from = NameFrom::Title,
                            Some(other) => r.error(
                                v.span().start,
                                format!(
                                    "name-from is directory, program or title, not \"{other}\""
                                ),
                            ),
                            None => {}
                        },
                        "name-follow" => {
                            if let Some(b) = r.boolean("name-follow", v) {
                                config.window_name_follow = b;
                            }
                        }
                        other => r.error(
                            n.span().start,
                            unknown_key(
                                "window",
                                other,
                                &["base-index", "renumber", "name-from", "name-follow"],
                            ),
                        ),
                    }
                }
            }
            "power" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "prevent-sleep" => {
                            if let Some(b) = r.boolean("prevent-sleep", v) {
                                config.prevent_sleep = b;
                            }
                        }
                        other => {
                            r.error(
                                n.span().start,
                                unknown_key("power", other, &["prevent-sleep"]),
                            );
                        }
                    }
                }
            }
            "upgrade" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "auto" => {
                            if let Some(b) = r.boolean("auto", v) {
                                config.upgrade_auto = b;
                            }
                        }
                        other => {
                            r.error(n.span().start, unknown_key("upgrade", other, &["auto"]));
                        }
                    }
                }
            }
            "session" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "on-end" => {
                            if let Some(text) = r.string("on-end", v) {
                                match text {
                                    "switch" => config.on_session_end = OnSessionEnd::Switch,
                                    "detach" => config.on_session_end = OnSessionEnd::Detach,
                                    other => r.error(
                                        v.span().start,
                                        format!(
                                            "on-end is \"switch\" or \"detach\", not \"{other}\""
                                        ),
                                    ),
                                }
                            }
                        }
                        other => {
                            r.error(n.span().start, unknown_key("session", other, &["on-end"]))
                        }
                    }
                }
            }
            "notification" => read_notification(&mut r, table, &mut config.notification),
            "scrollback" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "lines" => config.scrollback_lines = r.integer("lines", v),
                        "show-position" => {
                            if let Some(b) = r.boolean("show-position", v) {
                                config.scroll_position = b;
                            }
                        }
                        other => {
                            r.error(
                                n.span().start,
                                unknown_key("scrollback", other, &["lines", "show-position"]),
                            );
                        }
                    }
                }
            }
            "clipboard" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "mode" => match r.string("mode", v) {
                            Some("osc52") => config.clipboard.mode = ClipboardMode::Osc52,
                            Some("command") => config.clipboard.mode = ClipboardMode::Command,
                            Some("off") => config.clipboard.mode = ClipboardMode::Off,
                            Some(other) => r.error(
                                v.span().start,
                                format!("mode is osc52, command or off, not \"{other}\""),
                            ),
                            None => {}
                        },
                        "command" => {
                            config.clipboard.command = r.string("command", v).map(str::to_string);
                        }
                        "allow-pane-write" => {
                            if let Some(b) = r.boolean("allow-pane-write", v) {
                                config.clipboard.allow_pane_write = b;
                            }
                        }
                        other => {
                            r.error(
                                n.span().start,
                                unknown_key(
                                    "clipboard",
                                    other,
                                    &["mode", "command", "allow-pane-write"],
                                ),
                            );
                        }
                    }
                }
                if config.clipboard.mode == ClipboardMode::Command
                    && config.clipboard.command.is_none()
                {
                    r.error(
                        value.span().start,
                        "clipboard.mode = \"command\" needs command",
                    );
                }
            }
            "links" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "command" => match v.get_ref() {
                            DeValue::Array(items) if !items.is_empty() => {
                                config.links.command = Some(
                                    items
                                        .iter()
                                        .filter_map(|i| r.string("command", i).map(str::to_string))
                                        .collect(),
                                );
                            }
                            _ => r.error(
                                v.span().start,
                                "command must be a list of words, the program first (for example [\"open\"])",
                            ),
                        },
                        "underline" => {
                            if let Some(b) = r.boolean("underline", v) {
                                config.links.underline = b;
                            }
                        }
                        other => {
                            r.error(
                                n.span().start,
                                unknown_key("links", other, &["command", "underline"]),
                            );
                        }
                    }
                }
            }
            other => r.error(
                name.span().start,
                unknown_key(
                    "",
                    other,
                    &[
                        "links",
                        "keys",
                        "status",
                        "window",
                        "scrollback",
                        "clipboard",
                        "power",
                        "upgrade",
                        "session",
                        "notification",
                    ],
                ),
            ),
        }
    }
    config.keys.prefix = normalize(config.keys.prefix);
    if r.errors.is_empty() {
        Ok(config)
    } else {
        r.errors.sort_by_key(|e| e.line);
        Err(r.errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHOR: &str = r#"
[keys]
prefix = "ctrl+m"
direct-modifier = "ctrl+alt"
nav = "hnei"

[status]
position = "top"
"#;

    #[test]
    fn links_are_underlined_unless_the_file_says_not() {
        assert!(Config::default().links.underline);
        assert!(
            parse("[links]\nunderline = true\n")
                .unwrap()
                .links
                .underline
        );
        assert!(
            !parse("[links]\nunderline = false\n")
                .unwrap()
                .links
                .underline
        );
        assert!(parse("[links]\nunderline = \"yes\"\n").is_err());
    }

    #[test]
    fn what_happens_when_a_session_ends_is_set_by_on_end() {
        assert_eq!(Config::default().on_session_end, OnSessionEnd::Switch);
        assert_eq!(
            parse("[session]\non-end = \"detach\"\n")
                .unwrap()
                .on_session_end,
            OnSessionEnd::Detach
        );
        assert_eq!(
            parse("[session]\non-end = \"switch\"\n")
                .unwrap()
                .on_session_end,
            OnSessionEnd::Switch
        );
        let errors = parse("[session]\non-end = \"stay\"\n").unwrap_err();
        assert!(errors[0].message.contains("switch"), "{errors:?}");
    }

    #[test]
    fn the_authors_file_loads_without_errors() {
        let config = parse(AUTHOR).unwrap();
        assert_eq!(config.keys.nav, ['h', 'n', 'e', 'i']);
        assert_eq!(config.status_position, StatusPosition::Top);
        assert_eq!(config.keys.prefix, parse_key("ctrl+m").unwrap());
        let m = config.keymap();
        assert!(Keymap::get(&m.direct, parse_key("ctrl+alt+n").unwrap()).is_some());
    }

    #[test]
    fn the_prefix_setting_and_the_prefix_bindings_can_live_in_one_file() {
        let config =
            parse("[keys]\nprefix = \"ctrl+b\"\n[keys.after-prefix]\n\"q\" = \"pane.zoom\"\n")
                .unwrap();
        assert_eq!(config.keys.prefix, parse_key("ctrl+b").unwrap());
        assert!(Keymap::get(&config.keymap().prefix, parse_key("q").unwrap()).is_some());
        // As a table of its own, `keys.prefix` works when the prefix key keeps its default.
        let config = parse("[keys.prefix]\n\"q\" = \"pane.zoom\"\n").unwrap();
        assert!(Keymap::get(&config.keymap().prefix, parse_key("q").unwrap()).is_some());
    }

    #[test]
    fn an_empty_file_is_the_defaults() {
        assert_eq!(parse("").unwrap(), Config::default());
    }

    #[test]
    fn bindings_are_set_removed_and_made_repeatable() {
        let config = parse(
            r#"
[keys.prefix]
"x" = false
"q" = "pane.zoom"
"H" = { run = "pane.resize-left", repeat = true }
"w" = "window.select 3"

[keys.direct]
"a" = "window.next"
"left" = false
"#,
        )
        .unwrap();
        let m = config.keymap();
        assert!(Keymap::get(&m.prefix, parse_key("x").unwrap()).is_none());
        assert_eq!(
            Keymap::get(&m.prefix, parse_key("q").unwrap())
                .unwrap()
                .command,
            "pane.zoom"
        );
        assert!(
            Keymap::get(&m.prefix, parse_key("H").unwrap())
                .unwrap()
                .repeat
        );
        assert_eq!(
            Keymap::get(&m.prefix, parse_key("w").unwrap())
                .unwrap()
                .args,
            vec!["3".to_string()]
        );
        assert_eq!(
            Keymap::get(&m.direct, parse_key("ctrl+alt+a").unwrap())
                .unwrap()
                .command,
            "window.next"
        );
        assert!(Keymap::get(&m.direct, parse_key("ctrl+alt+left").unwrap()).is_none());
    }

    fn lines(text: &str) -> Vec<String> {
        parse(text)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn the_machine_is_kept_awake_unless_the_file_says_not() {
        assert!(Config::default().upgrade_auto);
        assert!(parse("[upgrade]\nauto = true\n").unwrap().upgrade_auto);
        assert!(!parse("[upgrade]\nauto = false\n").unwrap().upgrade_auto);
        assert!(!lines("[upgrade]\nauto = 3\n").is_empty());
        assert!(Config::default().prevent_sleep);
        assert!(parse("").unwrap().prevent_sleep);
        assert!(parse("[power]\n").unwrap().prevent_sleep);
        assert!(
            parse("[power]\nprevent-sleep = true\n")
                .unwrap()
                .prevent_sleep
        );
        assert!(
            !parse("[power]\nprevent-sleep = false\n")
                .unwrap()
                .prevent_sleep
        );
        let got = lines("[power]\nprevent-sleep = \"no\"\n");
        assert!(got[0].starts_with("line 2:"), "{got:?}");
        let got = lines("[power]\nprevent-slep = false\n");
        assert_eq!(
            got,
            vec![
                "line 2: unknown key \"power.prevent-slep\", did you mean \"power.prevent-sleep\"?"
            ]
        );
        let got = lines("[powr]\nprevent-sleep = false\n");
        assert_eq!(
            got,
            vec!["line 1: unknown section \"powr\", did you mean \"power\"?"]
        );
    }

    #[test]
    fn mistakes_come_with_their_line() {
        let got = lines("[keys]\nprefix = \"ctrl+a\"\nbogus = 1\n");
        assert_eq!(got, vec!["line 3: unknown key \"keys.bogus\""]);
        let got = lines("\n\n[nothing]\na = 1\n");
        assert_eq!(got, vec!["line 3: unknown section \"nothing\""]);
        // A near miss gets the closest name; a name like nothing known gets none.
        let got = lines("[keys]\nprefx = \"ctrl+b\"\nzzzzzz = 1\n[statu]\nposition = \"top\"\n");
        assert_eq!(
            got,
            vec![
                "line 2: unknown key \"keys.prefx\", did you mean \"keys.prefix\"?",
                "line 3: unknown key \"keys.zzzzzz\"",
                "line 4: unknown section \"statu\", did you mean \"status\"?",
            ]
        );
        let got = lines("[keys.prefix]\n\"z\" = \"pane.fly\"\n");
        assert_eq!(got.len(), 1);
        assert!(
            got[0].starts_with("line 2: unknown command \"pane.fly\""),
            "{got:?}"
        );
        let got = lines("[status]\nposition = \"middle\"\n");
        assert!(got[0].starts_with("line 2:"), "{got:?}");
        let got = lines("[keys\n");
        assert!(got[0].starts_with("line 1:"), "{got:?}");
        let got = lines("[window]\nbase-index = \"one\"\n");
        assert!(got[0].starts_with("line 2:"), "{got:?}");
        let got = lines("[keys]\nnav = \"hh\"\n");
        assert!(got[0].starts_with("line 2:"), "{got:?}");
    }

    #[test]
    fn two_bindings_of_one_key_name_both_lines() {
        let got = lines("[keys.prefix]\n\"H\" = \"pane.zoom\"\n\"shift+h\" = \"pane.close\"\n");
        assert_eq!(got.len(), 1);
        assert!(got[0].contains("lines 2 and 3"), "{got:?}");
        assert!(got[0].starts_with("line 3:"), "{got:?}");
    }

    #[test]
    fn the_copy_list_and_clipboard_tables_are_read_and_checked() {
        let config = parse(
            "[keys.copy]\n\"x\" = \"copy.exit\"\n[keys.list]\n\"q\" = false\n[clipboard]\nmode = \"command\"\ncommand = \"pbcopy\"\nallow-pane-write = true\n[scrollback]\nlines = 1000\n",
        )
        .unwrap();
        assert_eq!(config.clipboard.mode, ClipboardMode::Command);
        assert_eq!(config.scrollback_lines, Some(1000));
        assert!(config.clipboard.allow_pane_write);
        assert!(!config.keymap().copy.is_empty());
        let got = lines("[keys.copy]\n\"x\" = \"copy.fly\"\n");
        assert!(got[0].starts_with("line 2:"));
        let got = lines("[clipboard]\nmode = \"command\"\n");
        assert!(got[0].contains("needs command"), "{got:?}");
    }

    #[test]
    fn the_direct_table_needs_a_modifier() {
        let got =
            lines("[keys]\ndirect-modifier = \"none\"\n[keys.direct]\n\"a\" = \"window.next\"\n");
        assert!(got[0].starts_with("line 4:"), "{got:?}");
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_the_path_comes_from_the_environment() {
        let dir = std::env::temp_dir().join("arcade-config-missing-test");
        assert_eq!(load(&dir.join("nope.toml")).unwrap(), Config::default());
        assert_eq!(
            default_path(Some("/x/c.toml"), Some("/home/a")),
            Some(PathBuf::from("/x/c.toml"))
        );
        assert_eq!(
            default_path(None, Some("/home/a")),
            Some(PathBuf::from("/home/a/.config/arcade/config.toml"))
        );
        assert_eq!(default_path(None, None), None);
    }

    #[test]
    fn building_the_tables_ten_times_gives_the_same_tables() {
        let config = parse(AUTHOR).unwrap();
        let first = config.keymap();
        for _ in 0..10 {
            let again = parse(AUTHOR).unwrap().keymap();
            assert_eq!(again.prefix, first.prefix);
            assert_eq!(again.direct, first.direct);
        }
    }

    #[test]
    fn the_notification_settings_have_the_defaults_of_the_spec_and_can_be_changed() {
        let d = parse("").unwrap().notification;
        assert_eq!(d, NotificationConfig::default());
        assert!(d.auto_jump && d.push_when_host_focused);
        assert_eq!(d.push_interval, Duration::from_secs(5));
        assert_eq!(d.bell_interval, Duration::from_secs(2));
        // Approval, input, error ring and ask; info only asks and goes away when looked at.
        assert!(d.class(crate::notify::Class::Approval).bell);
        assert!(!d.class(crate::notify::Class::Info).bell);
        assert!(d.class(crate::notify::Class::Info).clear_on_focus);
        assert!(!d.class(crate::notify::Class::Error).clear_on_focus);
        let c = parse(
            "[notification]\nauto-jump = false\nosc = \"9\"\npush-interval = 7\nmute-sources = [\"a\", \"b\"]\n[notification.info]\nbell = true\n",
        )
        .unwrap()
        .notification;
        assert!(!c.auto_jump);
        assert_eq!(c.osc, OscKind::Osc9);
        assert_eq!(c.push_interval, Duration::from_secs(7));
        assert_eq!(c.muted_sources, ["a", "b"]);
        assert!(c.class(crate::notify::Class::Info).bell);
        assert!(c.class(crate::notify::Class::Info).clear_on_focus);
    }

    #[test]
    fn the_places_of_the_notification_marks_can_be_turned_off() {
        let d = parse("").unwrap().notification;
        assert!(d.tab_mark && d.pane_mark);
        assert_eq!(d.status_count, StatusCount::OtherSessions);
        let c =
            parse("[notification]\ntab-mark = false\npane-mark = false\nstatus-count = \"off\"\n")
                .unwrap()
                .notification;
        assert!(!c.tab_mark && !c.pane_mark);
        assert_eq!(c.status_count, StatusCount::Off);
        let c = parse("[notification]\nstatus-count = \"all\"\n")
            .unwrap()
            .notification;
        assert_eq!(c.status_count, StatusCount::All);
        let got = lines("[notification]\nstatus-count = \"some\"\n");
        assert!(got[0].contains("status-count is"), "{got:?}");
    }

    #[test]
    fn mistakes_in_the_notification_settings_are_named_with_their_line() {
        let got = lines("[notification]\nauto-jmp = true\n");
        assert_eq!(
            got,
            [
                "line 2: unknown key \"notification.auto-jmp\", did you mean \"notification.auto-jump\"?"
            ]
        );
        let got = lines("[notification.info]\nbel = true\n");
        assert_eq!(got.len(), 1);
        assert!(got[0].contains("notification.info.bel"), "{got:?}");
        let got = lines("[notification]\nosc = \"99\"\n");
        assert!(got[0].starts_with("line 2: osc is"), "{got:?}");
        let got = lines("[notification]\nmute-sources = \"a\"\n");
        assert!(got[0].contains("list of source names"), "{got:?}");
    }

    #[test]
    fn the_tab_style_and_the_number_place_have_defaults_and_a_clear_error() {
        let config = parse("").unwrap();
        assert_eq!(config.tab_style, TabStyle::Pill);
        assert_eq!(config.tab_number, NumberPosition::Right);
        let flat = parse("[status]\ntab-style = \"flat\"\nnumber = \"left\"\n").unwrap();
        assert_eq!(flat.tab_style, TabStyle::Flat);
        assert_eq!(flat.tab_number, NumberPosition::Left);
        let errors = parse("[status]\ntab-style = \"round\"\n").unwrap_err();
        assert!(errors[0].message.contains("pill or flat"), "{errors:?}");
    }

    #[test]
    fn the_times_of_the_command_states_have_defaults_and_zero_turns_them_off() {
        let config = parse("").unwrap();
        assert_eq!(config.command_running_after, Duration::from_secs(2));
        assert_eq!(config.command_finished_after, Duration::from_secs(10));
        let config = parse("[status]\nrunning-after = 5\nfinished-after = 0\n").unwrap();
        assert_eq!(config.command_running_after, Duration::from_secs(5));
        assert_eq!(config.command_finished_after, Duration::ZERO);
        let errors = parse("[status]\nrunning-after = \"soon\"\n").unwrap_err();
        assert!(errors[0].message.contains("whole number"), "{errors:?}");
    }
}
