//! The key tables: which key runs which command.
//!
//! Four tables (`direct`, `prefix`, `copy`, `list`) map a key to a named command. Nothing in the
//! code that handles keys knows a particular key; the defaults are built here from a few
//! settings (the prefix key, the modifier of the direct table, the four movement characters),
//! and every binding can be replaced the same way.

use std::collections::HashMap;
use std::time::Duration;

use arcade_term::{Code, Key, Mods, Named};

use crate::layout::Direction;

/// The settings the default tables are built from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeysConfig {
    /// The key that starts the prefix table.
    pub prefix: Key,
    /// The modifier of every binding in the direct table; `None` turns that table off.
    pub direct_modifier: Option<Mods>,
    /// The movement keys: left, down, up, right.
    pub nav: [char; 4],
    /// How long the prefix table stays active after a repeatable binding.
    pub repeat_window: Duration,
}

impl Default for KeysConfig {
    fn default() -> Self {
        KeysConfig {
            prefix: Key {
                code: Code::Char('a'),
                mods: Mods {
                    ctrl: true,
                    ..Mods::NONE
                },
            },
            direct_modifier: Some(Mods {
                alt: true,
                ..Mods::NONE
            }),
            nav: ['h', 'j', 'k', 'l'],
            repeat_window: Duration::from_millis(600),
        }
    }
}

/// What a key does: a command with its arguments, and whether the prefix table stays active
/// for the repeat window afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub command: String,
    pub args: Vec<String>,
    pub repeat: bool,
}

pub type Table = HashMap<Key, Binding>;

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    pub direct: Table,
    pub prefix: Table,
    pub copy: Table,
    pub list: Table,
}

/// The key as it is looked up. A shifted symbol is the same key whether the host sent the
/// symbol (`>`) or the base key with Shift (`.` and Shift), and a letter is its lower case
/// with Shift. (The symbols are those of the US layout, which is what terminals report.)
pub fn normalize(key: Key) -> Key {
    let Code::Char(c) = key.code else {
        return key;
    };
    let base = match c {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        _ => return key,
    };
    Key {
        code: Code::Char(base),
        mods: Mods {
            shift: true,
            ..key.mods
        },
    }
}

fn named_key(name: &str) -> Option<Named> {
    Some(match name {
        "enter" | "return" => Named::Enter,
        "tab" => Named::Tab,
        "esc" | "escape" => Named::Escape,
        "backspace" => Named::Backspace,
        "up" => Named::Up,
        "down" => Named::Down,
        "left" => Named::Left,
        "right" => Named::Right,
        "home" => Named::Home,
        "end" => Named::End,
        "pageup" => Named::PageUp,
        "pagedown" => Named::PageDown,
        "insert" => Named::Insert,
        "delete" => Named::Delete,
        other => {
            let n: u8 = other.strip_prefix('f')?.parse().ok()?;
            if !(1..=12).contains(&n) {
                return None;
            }
            Named::F(n)
        }
    })
}

/// Read a key written as text: `a`, `H`, `>`, `ctrl+a`, `alt+shift+left`, `space`, `f5`.
/// The result is in the form [`normalize`] gives.
pub fn parse_key(text: &str) -> Result<Key, String> {
    let mut mods = Mods::NONE;
    let mut rest = text;
    // A modifier is a word followed by `+`; the key itself may be `+`.
    while let Some((word, after)) = rest.split_once('+') {
        if after.is_empty() {
            break;
        }
        match word.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "option" | "meta" => mods.alt = true,
            "shift" => mods.shift = true,
            "super" | "cmd" | "command" => mods.super_key = true,
            _ => return Err(format!("unknown modifier \"{word}\" in key \"{text}\"")),
        }
        rest = after;
    }
    let code = if rest.chars().count() == 1 {
        let c = rest.chars().next().unwrap_or(' ');
        if c.is_uppercase() {
            mods.shift = true;
            Code::Char(c.to_lowercase().next().unwrap_or(c))
        } else {
            Code::Char(c)
        }
    } else if rest.eq_ignore_ascii_case("space") {
        Code::Char(' ')
    } else if let Some(named) = named_key(&rest.to_ascii_lowercase()) {
        Code::Named(named)
    } else {
        return Err(format!("unknown key \"{rest}\" in \"{text}\""));
    };
    Ok(normalize(Key { code, mods }))
}

/// The words for a key, as `parse_key` reads them.
pub fn key_name(key: Key) -> String {
    let mut out = String::new();
    for (on, word) in [
        (key.mods.ctrl, "ctrl+"),
        (key.mods.alt, "alt+"),
        (key.mods.super_key, "super+"),
        (key.mods.shift, "shift+"),
    ] {
        if on {
            out.push_str(word);
        }
    }
    match key.code {
        Code::Char(' ') => out.push_str("space"),
        Code::Char(c) => out.push(c),
        Code::Named(n) => out.push_str(&match n {
            Named::Enter => "enter".to_string(),
            Named::Tab => "tab".to_string(),
            Named::Escape => "esc".to_string(),
            Named::Backspace => "backspace".to_string(),
            Named::Up => "up".to_string(),
            Named::Down => "down".to_string(),
            Named::Left => "left".to_string(),
            Named::Right => "right".to_string(),
            Named::Home => "home".to_string(),
            Named::End => "end".to_string(),
            Named::PageUp => "pageup".to_string(),
            Named::PageDown => "pagedown".to_string(),
            Named::Insert => "insert".to_string(),
            Named::Delete => "delete".to_string(),
            Named::F(n) => format!("f{n}"),
        }),
    }
    out
}

const DIRECTIONS: [Direction; 4] = [
    Direction::Left,
    Direction::Down,
    Direction::Up,
    Direction::Right,
];

fn direction_word(d: Direction) -> &'static str {
    match d {
        Direction::Left => "left",
        Direction::Down => "down",
        Direction::Up => "up",
        Direction::Right => "right",
    }
}

fn arrow(d: Direction) -> Named {
    match d {
        Direction::Left => Named::Left,
        Direction::Down => Named::Down,
        Direction::Up => Named::Up,
        Direction::Right => Named::Right,
    }
}

/// The two ways to say a direction: its movement character and its arrow.
fn direction_codes(config: &KeysConfig, index: usize) -> [Code; 2] {
    [
        Code::Char(config.nav[index]),
        Code::Named(arrow(DIRECTIONS[index])),
    ]
}

fn bind(table: &mut Table, key: Key, command: &str, args: &[&str], repeat: bool) {
    table.insert(
        normalize(key),
        Binding {
            command: command.to_string(),
            args: args.iter().map(|a| (*a).to_string()).collect(),
            repeat,
        },
    );
}

fn plain(code: Code) -> Key {
    Key {
        code,
        mods: Mods::NONE,
    }
}

fn ch(c: char) -> Key {
    plain(Code::Char(c))
}

fn with_shift(mut key: Key) -> Key {
    key.mods.shift = true;
    key
}

fn with_mods(key: Key, mods: Mods) -> Key {
    Key {
        code: key.code,
        mods: Mods {
            shift: key.mods.shift || mods.shift,
            alt: key.mods.alt || mods.alt,
            ctrl: key.mods.ctrl || mods.ctrl,
            super_key: key.mods.super_key || mods.super_key,
        },
    }
}

impl Keymap {
    /// The tables Arcade starts with, for these settings.
    pub fn defaults(config: &KeysConfig) -> Keymap {
        let mut map = Keymap::default();
        map.fill_direct(config);
        map.fill_prefix(config);
        map
    }

    fn fill_direct(&mut self, config: &KeysConfig) {
        let Some(m) = config.direct_modifier else {
            return;
        };
        let t = &mut self.direct;
        for (i, d) in DIRECTIONS.iter().enumerate() {
            for code in direction_codes(config, i) {
                let key = with_mods(plain(code), m);
                bind(
                    t,
                    key,
                    &format!("pane.focus-{}", direction_word(*d)),
                    &[],
                    false,
                );
                bind(
                    t,
                    with_shift(key),
                    &format!("pane.resize-{}", direction_word(*d)),
                    &[],
                    false,
                );
            }
        }
        for n in 1..=9u32 {
            let digit = char::from_digit(n, 10).unwrap_or('1');
            bind(
                t,
                with_mods(ch(digit), m),
                "window.select",
                &[&n.to_string()],
                false,
            );
        }
        bind(t, with_mods(ch(','), m), "window.previous", &[], false);
        bind(t, with_mods(ch('.'), m), "window.next", &[], false);
        bind(t, with_mods(ch('o'), m), "pane.focus-last", &[], false);
        bind(t, with_mods(ch('g'), m), "arcade.lock", &[], false);
    }

    fn fill_prefix(&mut self, config: &KeysConfig) {
        let t = &mut self.prefix;
        for (i, d) in DIRECTIONS.iter().enumerate() {
            for code in direction_codes(config, i) {
                bind(
                    t,
                    plain(code),
                    &format!("pane.focus-{}", direction_word(*d)),
                    &[],
                    true,
                );
                bind(
                    t,
                    with_shift(plain(code)),
                    &format!("pane.resize-{}", direction_word(*d)),
                    &[],
                    true,
                );
            }
        }
        bind(t, ch('|'), "pane.split-right", &[], false);
        bind(t, ch('-'), "pane.split-down", &[], false);
        bind(t, ch(';'), "pane.focus-last", &[], false);
        bind(t, ch('z'), "pane.zoom", &[], false);
        bind(t, ch('x'), "pane.close", &[], false);
        bind(t, ch('c'), "window.new", &[], false);
        for n in 1..=9u32 {
            let digit = char::from_digit(n, 10).unwrap_or('1');
            bind(t, ch(digit), "window.select", &[&n.to_string()], false);
        }
        bind(t, plain(Code::Named(Named::Tab)), "window.last", &[], false);
        bind(t, ch('u'), "window.next", &[], true);
        bind(t, ch('y'), "window.previous", &[], true);
        bind(t, ch('<'), "window.move-left", &[], true);
        bind(t, ch('>'), "window.move-right", &[], true);
        bind(t, ch(','), "window.rename", &[], false);
        bind(t, ch('$'), "session.rename", &[], false);
        bind(t, ch('d'), "session.detach", &[], false);
        bind(t, ch('g'), "arcade.lock", &[], false);
        bind(t, ch('R'), "arcade.redraw", &[], false);
    }

    /// The binding of `key` in `table`.
    pub fn get(table: &Table, key: Key) -> Option<&Binding> {
        table.get(&normalize(key))
    }

    /// Whether some key in the direct table unlocks (and locks).
    pub fn has_lock_key(&self) -> bool {
        self.direct.values().any(|b| b.command == "arcade.lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(text: &str) -> Key {
        parse_key(text).unwrap()
    }

    #[test]
    fn keys_are_read_from_words() {
        assert_eq!(
            k("ctrl+a"),
            Key {
                code: Code::Char('a'),
                mods: Mods {
                    ctrl: true,
                    ..Mods::NONE
                }
            }
        );
        assert_eq!(k("H"), k("shift+h"));
        assert_eq!(k(">"), k("shift+."));
        assert!(k("alt+shift+left").mods.shift);
        assert_eq!(k("space").code, Code::Char(' '));
        assert_eq!(k("f5").code, Code::Named(Named::F(5)));
        assert!(parse_key("hyper+a").is_err());
        assert!(parse_key("nonsense").is_err());
        assert_eq!(key_name(k("ctrl+alt+left")), "ctrl+alt+left");
    }

    #[test]
    fn a_shifted_symbol_is_one_key_in_both_forms() {
        let as_symbol = normalize(plain(Code::Char('>')));
        let as_base = normalize(with_shift(plain(Code::Char('.'))));
        assert_eq!(as_symbol, as_base);
    }

    #[test]
    fn the_defaults_follow_the_settings() {
        let config = KeysConfig {
            nav: ['h', 'n', 'e', 'i'],
            direct_modifier: Some(Mods {
                ctrl: true,
                alt: true,
                ..Mods::NONE
            }),
            ..KeysConfig::default()
        };
        let map = Keymap::defaults(&config);
        let focus_down = Keymap::get(&map.direct, k("ctrl+alt+n")).unwrap();
        assert_eq!(focus_down.command, "pane.focus-down");
        assert!(Keymap::get(&map.direct, k("alt+n")).is_none());
        assert_eq!(
            Keymap::get(&map.prefix, k("N")).unwrap().command,
            "pane.resize-down"
        );
        assert!(Keymap::get(&map.prefix, k("N")).unwrap().repeat);
        assert_eq!(
            Keymap::get(&map.prefix, k(">")).unwrap().command,
            "window.move-right"
        );
        assert!(map.has_lock_key());
        let off = Keymap::defaults(&KeysConfig {
            direct_modifier: None,
            ..KeysConfig::default()
        });
        assert!(off.direct.is_empty());
        assert!(!off.has_lock_key());
    }

    #[test]
    fn every_default_command_is_a_known_command() {
        let map = Keymap::defaults(&KeysConfig::default());
        for table in [&map.direct, &map.prefix] {
            for (key, b) in table {
                assert!(
                    crate::commands::parse(&b.command, &b.args).is_ok(),
                    "{key:?} -> {}",
                    b.command
                );
            }
        }
    }

    #[test]
    fn the_movement_characters_are_used_by_no_other_binding() {
        // `h j k l n e i`, lower case or upper case, belong to movement only.
        let map = Keymap::defaults(&KeysConfig::default());
        for c in "jkl"
            .chars()
            .chain("hjkl".chars().map(|c| c.to_ascii_uppercase()))
        {
            if let Some(b) = Keymap::get(&map.prefix, k(&c.to_string())) {
                assert!(
                    b.command.starts_with("pane.focus") || b.command.starts_with("pane.resize"),
                    "{c} -> {}",
                    b.command
                );
            }
        }
        for c in "nei".chars() {
            assert!(Keymap::get(&map.prefix, k(&c.to_string())).is_none(), "{c}");
        }
    }
}
