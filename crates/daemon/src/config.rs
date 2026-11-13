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
use crate::keymap::{Binding, Keymap, KeysConfig, Table, normalize, parse_key};

/// Where the status bar sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusPosition {
    Bottom,
    Top,
    Off,
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

/// The four key tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TableName {
    Direct,
    Prefix,
    Copy,
    List,
}

impl TableName {
    fn word(self) -> &'static str {
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

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub keys: KeysConfig,
    pub overrides: Vec<KeyOverride>,
    pub status_position: StatusPosition,
    /// The number of the first window.
    pub base_index: usize,
    /// Whether the windows' numbers close up when one goes.
    pub renumber: bool,
    pub scrollback_lines: Option<usize>,
    pub clipboard: ClipboardConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            keys: KeysConfig::default(),
            overrides: Vec::new(),
            status_position: StatusPosition::Bottom,
            base_index: 1,
            renumber: true,
            scrollback_lines: None,
            clipboard: ClipboardConfig {
                mode: ClipboardMode::Osc52,
                command: None,
                allow_pane_write: false,
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
            "repeat-window-ms" => {
                if let Some(ms) = r.integer("repeat-window-ms", value) {
                    config.keys.repeat_window = Duration::from_millis(ms as u64);
                }
            }
            "direct" | "after-prefix" | "copy" | "list" => {}
            _ => r.error(name.span().start, format!("unknown key \"keys.{key}\"")),
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
                Ok((command, args)) => Some(Binding {
                    command,
                    args,
                    repeat,
                }),
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
                        other => r.error(n.span().start, format!("unknown key \"status.{other}\"")),
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
                        other => r.error(n.span().start, format!("unknown key \"window.{other}\"")),
                    }
                }
            }
            "scrollback" => {
                for (n, v) in table.iter() {
                    match n.get_ref().as_ref() {
                        "lines" => config.scrollback_lines = r.integer("lines", v),
                        other => {
                            r.error(
                                n.span().start,
                                format!("unknown key \"scrollback.{other}\""),
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
                            r.error(n.span().start, format!("unknown key \"clipboard.{other}\""));
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
            other => r.error(name.span().start, format!("unknown section \"{other}\"")),
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
            Keymap::get(&m.direct, parse_key("alt+a").unwrap())
                .unwrap()
                .command,
            "window.next"
        );
        assert!(Keymap::get(&m.direct, parse_key("alt+left").unwrap()).is_none());
    }

    fn lines(text: &str) -> Vec<String> {
        parse(text)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn mistakes_come_with_their_line() {
        let got = lines("[keys]\nprefix = \"ctrl+a\"\nbogus = 1\n");
        assert_eq!(got, vec!["line 3: unknown key \"keys.bogus\""]);
        let got = lines("\n\n[nothing]\na = 1\n");
        assert_eq!(got, vec!["line 3: unknown section \"nothing\""]);
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
}
