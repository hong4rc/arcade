//! What the key tables hold, and which bindings cannot work: the text of `arcade keys list` and
//! the warnings of `arcade config check`, `arcade keys list` and the status bar.
//!
//! Both read a [`Config`] and nothing else, so the same file always gives the same text.

use std::fmt;

use arcade_term::{Code, Key, Named};

use crate::config::{Config, TableName};
use crate::keymap::{Binding, Keymap, KeysConfig, Table, key_name, normalize, typed_name};

/// What the host terminal can send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    /// It has the kitty keyboard protocol: every combination can be told apart.
    Kitty,
    /// It sends the old key codes only.
    Legacy,
}

/// Where a binding comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    Config,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::Config => "config",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: Key,
    pub binding: Binding,
    pub source: Source,
}

const TABLES: [TableName; 4] = [
    TableName::Direct,
    TableName::Prefix,
    TableName::Copy,
    TableName::List,
];

fn table_of(map: &Keymap, name: TableName) -> &Table {
    match name {
        TableName::Direct => &map.direct,
        TableName::Prefix => &map.prefix,
        TableName::Copy => &map.copy,
        TableName::List => &map.list,
    }
}

/// The text of a binding's command and arguments.
fn action(binding: &Binding) -> String {
    if binding.args.is_empty() {
        binding.command.clone()
    } else {
        format!("{} {}", binding.command, binding.args.join(" "))
    }
}

/// Every binding in force, table by table, in a fixed order: by command, then by key name.
pub fn listing(config: &Config) -> Vec<(TableName, Vec<Entry>)> {
    let map = config.keymap();
    let stock = Keymap::defaults(&KeysConfig::default());
    TABLES
        .iter()
        .map(|name| {
            let stock_table = table_of(&stock, *name);
            let mut entries: Vec<Entry> = table_of(&map, *name)
                .iter()
                .map(|(key, binding)| Entry {
                    key: *key,
                    binding: binding.clone(),
                    source: if stock_table.get(key) == Some(binding) {
                        Source::Default
                    } else {
                        Source::Config
                    },
                })
                .collect();
            entries.sort_by_key(|e| (action(&e.binding), key_name(e.key)));
            (*name, entries)
        })
        .collect()
}

/// The text `arcade keys list` prints (without the warnings).
pub fn list_lines(config: &Config) -> Vec<String> {
    let mut lines = Vec::new();
    for (name, entries) in listing(config) {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("[{}]", name.word()));
        let keys: Vec<String> = entries.iter().map(|e| typed_name(e.key)).collect();
        let actions: Vec<String> = entries
            .iter()
            .map(|e| {
                let mut text = action(&e.binding);
                if e.binding.repeat {
                    text.push_str(" (repeats)");
                }
                text
            })
            .collect();
        let key_width = keys.iter().map(|k| k.chars().count()).max().unwrap_or(0);
        let action_width = actions.iter().map(|a| a.chars().count()).max().unwrap_or(0);
        if entries.is_empty() {
            lines.push("  (no bindings)".to_string());
        }
        for ((entry, key), action) in entries.iter().zip(&keys).zip(&actions) {
            lines.push(format!(
                "  {key:<key_width$}  {action:<action_width$}  {}",
                entry.source.word()
            ));
        }
    }
    lines
}

/// What to write to the host terminal to learn whether it has the kitty keyboard protocol: ask
/// for the protocol's flags (`CSI ? u`), then for the terminal's attributes (`CSI c`), which every
/// terminal answers. A terminal that has the protocol answers the first; the answer to the
/// second comes after it, so when it arrives without the first the protocol is not there.
pub const HOST_QUERY: &[u8] = b"\x1b[?u\x1b[c";

/// An answer to [`HOST_QUERY`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// `CSI ? flags u`: the protocol is there.
    Keyboard,
    /// `CSI ? attributes c`: the last answer to come.
    Attributes,
}

/// Whether `raw` (one escape sequence read from the host terminal) is an answer to the query.
pub fn read_reply(raw: &[u8]) -> Option<Reply> {
    let body = raw.strip_prefix(b"\x1b[?")?;
    let (last, params) = body.split_last()?;
    if !params.iter().all(|b| b.is_ascii_digit() || *b == b';') {
        return None;
    }
    match last {
        b'u' if !params.is_empty() => Some(Reply::Keyboard),
        b'c' => Some(Reply::Attributes),
        _ => None,
    }
}

/// A binding (or the prefix key) that cannot work, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// `keys.direct`, `keys.prefix`, ... or `keys` for the prefix key itself.
    pub place: String,
    pub key: Key,
    pub reason: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}: {}",
            self.place,
            typed_name(self.key),
            self.reason
        )
    }
}

/// Why the old key codes cannot carry `key`, if they cannot.
fn legacy_problem(key: Key) -> Option<String> {
    let m = key.mods;
    if m.super_key {
        return Some(
            "the terminal does not send the Super key without the keyboard protocol".into(),
        );
    }
    match key.code {
        Code::Named(Named::Enter | Named::Escape | Named::Backspace) if m.ctrl || m.shift => {
            Some("the terminal sends it as the key without Ctrl or Shift".into())
        }
        Code::Named(Named::Tab) if m.ctrl => {
            Some("the terminal sends it as Tab without Ctrl".into())
        }
        Code::Named(_) => None,
        Code::Char(_) if !m.ctrl => None,
        Code::Char(_) if m.shift => {
            Some("the terminal sends Ctrl+Shift+key the same as Ctrl+key".into())
        }
        Code::Char('m') => Some(if m.alt {
            "the terminal sends it as Alt+Enter".into()
        } else {
            "the terminal sends it as Enter".into()
        }),
        Code::Char('i') => Some(if m.alt {
            "the terminal sends it as Alt+Tab".into()
        } else {
            "the terminal sends it as Tab".into()
        }),
        Code::Char('[') => Some("the terminal sends it as Esc".into()),
        Code::Char(c) if c.is_ascii_digit() => {
            Some("the terminal does not send Ctrl with a digit".into())
        }
        Code::Char(',' | '.' | ';' | '\'' | '=' | '-' | '`') => {
            Some("the terminal does not send Ctrl with this symbol".into())
        }
        Code::Char(_) => None,
    }
}

/// The bindings that cannot work. `host` is what the host terminal can send, when it is known:
/// the problems of the old key codes are only reported for `Some(Host::Legacy)`.
pub fn warnings(config: &Config, host: Option<Host>) -> Vec<Warning> {
    warnings_on(config, host, cfg!(target_os = "macos"))
}

fn warnings_on(config: &Config, host: Option<Host>, macos: bool) -> Vec<Warning> {
    let map = config.keymap();
    let prefix = normalize(config.keys.prefix);
    let mut out = Vec::new();
    let mut check = |place: &str, key: Key| {
        if macos && key.code == Code::Char(' ') && key.mods.ctrl {
            out.push(Warning {
                place: place.to_string(),
                key,
                reason: "macOS keeps Ctrl+Space for switching the input source, so it never reaches the terminal"
                    .into(),
            });
        }
        if host == Some(Host::Legacy)
            && let Some(reason) = legacy_problem(key)
        {
            out.push(Warning {
                place: place.to_string(),
                key,
                reason,
            });
        }
    };
    check("keys", prefix);
    for name in TABLES {
        let place = format!("keys.{}", name.word());
        let mut keys: Vec<Key> = table_of(&map, name).keys().copied().collect();
        keys.sort_by_key(|k| key_name(*k));
        for key in keys {
            if key != prefix {
                check(&place, key);
            }
        }
    }
    if map.direct.contains_key(&prefix) {
        out.push(Warning {
            place: "keys.direct".into(),
            key: prefix,
            reason: "this is the prefix key, which is read first, so the binding is never reached"
                .into(),
        });
    }
    if map.prefix.contains_key(&prefix) {
        out.push(Warning {
            place: "keys.prefix".into(),
            key: prefix,
            reason: "this is the prefix key: pressing it twice no longer sends it to the program"
                .into(),
        });
    }
    out
}

/// The one line for the status bar, or nothing when all is well.
pub fn status_line(warnings: &[Warning]) -> Option<String> {
    let first = warnings.first()?;
    let count = warnings.len();
    Some(format!(
        "{count} key binding{} cannot work here (first: {}); run arcade keys list",
        if count == 1 { "" } else { "s" },
        typed_name(first.key)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn parse(text: &str) -> Config {
        config::parse(text).unwrap()
    }

    fn found(config: &Config, host: Option<Host>, place: &str, key: &str) -> bool {
        let key = crate::keymap::parse_key(key).unwrap();
        warnings_on(config, host, false)
            .iter()
            .any(|w| w.place == place && w.key == key)
    }

    #[test]
    fn the_default_tables_are_listed_with_their_source_and_a_stable_order() {
        let config = Config::default();
        let first = list_lines(&config);
        assert!(first[0] == "[direct]", "{first:?}");
        assert!(first.iter().any(|l| l.starts_with("[prefix]")));
        assert!(
            first
                .iter()
                .any(|l| l.contains("pane.focus-left") && l.ends_with("default")),
            "{first:?}"
        );
        assert!(first.iter().all(|l| !l.ends_with("config")), "{first:?}");
        for _ in 0..10 {
            assert_eq!(list_lines(&Config::default()), first);
        }
    }

    #[test]
    fn a_binding_the_file_makes_or_changes_comes_from_the_config() {
        let config = parse("[keys.prefix]\n\"b\" = \"pane.zoom\"\n\"x\" = false\n");
        let prefix = listing(&config)
            .into_iter()
            .find(|(n, _)| *n == TableName::Prefix)
            .unwrap()
            .1;
        let b = prefix
            .iter()
            .find(|e| e.key == crate::keymap::parse_key("b").unwrap())
            .unwrap();
        assert_eq!(b.source, Source::Config);
        // A removed default is not in force, so it is not listed.
        assert!(
            prefix
                .iter()
                .all(|e| e.key != crate::keymap::parse_key("x").unwrap())
        );
        let z = prefix
            .iter()
            .find(|e| e.key == crate::keymap::parse_key("z").unwrap())
            .unwrap();
        assert_eq!(z.source, Source::Default);
    }

    #[test]
    fn a_setting_that_changes_the_defaults_makes_those_bindings_come_from_the_config() {
        let config = parse("[keys]\nnav = \"hnei\"\n");
        let lines = list_lines(&config);
        let line = lines
            .iter()
            .find(|l| l.contains("pane.focus-left") && l.trim_start().starts_with("ctrl+alt+h"))
            .unwrap();
        assert!(line.ends_with("default"), "{line}");
        let moved = lines
            .iter()
            .find(|l| l.contains("pane.focus-down") && l.trim_start().starts_with("ctrl+alt+n"))
            .unwrap();
        assert!(moved.ends_with("config"), "{moved}");
    }

    #[test]
    fn a_terminal_with_the_new_protocol_has_no_problems_with_old_key_codes() {
        let config = parse("[keys.direct]\n\"ctrl+1\" = \"pane.zoom\"\n");
        assert!(warnings_on(&config, Some(Host::Kitty), false).is_empty());
        // And when the terminal is not known, nothing is claimed about it.
        assert!(warnings_on(&config, None, false).is_empty());
    }

    #[test]
    fn a_terminal_with_old_key_codes_cannot_send_ctrl_with_a_digit() {
        let config =
            parse("[keys]\ndirect-modifier = \"ctrl\"\n[keys.direct]\n\"5\" = \"pane.zoom\"\n");
        assert!(found(&config, Some(Host::Legacy), "keys.direct", "ctrl+5"));
        let text = warnings_on(&config, Some(Host::Legacy), false)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("ctrl+5"), "{text}");
        assert!(text.contains("digit"), "{text}");
    }

    #[test]
    fn the_default_prefix_is_enter_on_a_terminal_with_old_key_codes() {
        let config = Config::default();
        assert!(found(&config, Some(Host::Legacy), "keys", "ctrl+m"));
        let ok = parse("[keys]\nprefix = \"ctrl+g\"\n");
        assert!(!found(&ok, Some(Host::Legacy), "keys", "ctrl+g"));
    }

    #[test]
    fn the_default_direct_digits_lose_their_ctrl_on_old_key_codes() {
        let config = Config::default();
        for key in ["ctrl+alt+1", "ctrl+alt+9", "ctrl+alt+,", "ctrl+alt+."] {
            assert!(
                found(&config, Some(Host::Legacy), "keys.direct", key),
                "{key}"
            );
        }
        // The letters and the arrows are carried.
        for key in [
            "ctrl+alt+h",
            "ctrl+alt+left",
            "ctrl+alt+shift+left",
            "ctrl+alt+a",
        ] {
            assert!(
                !found(&config, Some(Host::Legacy), "keys.direct", key),
                "{key}"
            );
        }
    }

    #[test]
    fn keys_the_old_codes_make_the_same_as_other_keys_are_named() {
        let config = parse(
            "[keys.prefix]\n\"ctrl+i\" = \"pane.zoom\"\n\"ctrl+[\" = \"pane.zoom\"\n\"ctrl+shift+b\" = \"pane.zoom\"\n\"ctrl+enter\" = \"pane.zoom\"\n\"super+k\" = \"pane.zoom\"\n\"alt+enter\" = \"pane.zoom\"\n\"shift+tab\" = \"pane.zoom\"\n",
        );
        for key in ["ctrl+i", "ctrl+[", "ctrl+shift+b", "ctrl+enter", "super+k"] {
            assert!(
                found(&config, Some(Host::Legacy), "keys.prefix", key),
                "{key}"
            );
        }
        for key in ["alt+enter", "shift+tab"] {
            assert!(
                !found(&config, Some(Host::Legacy), "keys.prefix", key),
                "{key}"
            );
        }
    }

    #[test]
    fn a_direct_binding_on_the_prefix_key_is_never_reached() {
        let config =
            parse("[keys]\nprefix = \"ctrl+alt+x\"\n[keys.direct]\n\"x\" = \"pane.zoom\"\n");
        let all = warnings_on(&config, None, false);
        assert!(
            all.iter()
                .any(|w| w.place == "keys.direct" && w.reason.contains("prefix key")),
            "{all:?}"
        );
    }

    #[test]
    fn a_prefix_binding_on_the_prefix_key_stops_it_being_sent_through() {
        let config =
            parse("[keys]\nprefix = \"ctrl+g\"\n[keys.after-prefix]\n\"ctrl+g\" = \"pane.zoom\"\n");
        let all = warnings_on(&config, None, false);
        assert!(
            all.iter()
                .any(|w| w.place == "keys.prefix" && w.reason.contains("twice")),
            "{all:?}"
        );
    }

    #[test]
    fn ctrl_space_is_reported_on_macos_only() {
        let config = parse("[keys]\nprefix = \"ctrl+space\"\n");
        assert!(
            warnings_on(&config, None, true)
                .iter()
                .any(|w| w.reason.contains("macOS"))
        );
        assert!(warnings_on(&config, None, false).is_empty());
    }

    #[test]
    fn the_default_configuration_has_no_warnings_that_do_not_depend_on_the_host() {
        assert!(warnings_on(&Config::default(), None, true).is_empty());
        assert!(warnings_on(&Config::default(), Some(Host::Kitty), true).is_empty());
    }

    #[test]
    fn the_answers_to_the_host_query_are_told_apart_from_other_sequences() {
        assert_eq!(read_reply(b"\x1b[?0u"), Some(Reply::Keyboard));
        assert_eq!(read_reply(b"\x1b[?31u"), Some(Reply::Keyboard));
        assert_eq!(read_reply(b"\x1b[?62;4;22c"), Some(Reply::Attributes));
        assert_eq!(read_reply(b"\x1b[?c"), Some(Reply::Attributes));
        for other in [
            &b"\x1b[?u"[..],
            b"\x1b[>0c",
            b"\x1b[65;5u",
            b"\x1b[A",
            b"\x1b[?2004;1$y",
            b"x",
        ] {
            assert_eq!(read_reply(other), None, "{other:?}");
        }
    }

    #[test]
    fn the_status_line_counts_the_warnings_and_names_the_first() {
        let config = Config::default();
        let all = warnings_on(&config, Some(Host::Legacy), false);
        let line = status_line(&all).unwrap();
        assert!(
            line.starts_with(&format!("{} key bindings", all.len())),
            "{line}"
        );
        assert!(line.contains(&typed_name(all[0].key)), "{line}");
        assert_eq!(status_line(&[]), None);
    }
}
