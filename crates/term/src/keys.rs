//! Keys as Arcade's daemon sees them, and their encoding for a pane's program.
//!
//! The daemon parses what the host terminal sends into a [`Key`]; when the key is not one Arcade
//! acts on, [`encode`] turns it into the bytes the pane's program asked for (it may have switched
//! on application cursor keys or the kitty keyboard protocol). The encoding itself is done by
//! libghostty-vt, which this crate alone calls.

use libghostty_vt::key::{Encoder, Event, Key as GKey, KittyKeyFlags, Mods as GMods, OptionAsAlt};

/// A key that is not a character.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Named {
    Enter,
    Tab,
    Escape,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// A function key, 1 to 12.
    F(u8),
}

/// What key was pressed, apart from the modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Code {
    /// A character, given without Shift applied for letters (`a`, not `A`).
    Char(char),
    Named(Named),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_key: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: false,
        super_key: false,
    };

    pub fn is_empty(self) -> bool {
        self == Mods::NONE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: Code,
    pub mods: Mods,
}

/// What the pane's program asked for, read from its terminal state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneKeyModes {
    pub app_cursor_keys: bool,
    /// The kitty keyboard flags the program set (zero when it did not ask for the protocol).
    pub kitty_flags: u8,
}

fn physical(c: char) -> Option<GKey> {
    Some(match c.to_ascii_lowercase() {
        'a' => GKey::A,
        'b' => GKey::B,
        'c' => GKey::C,
        'd' => GKey::D,
        'e' => GKey::E,
        'f' => GKey::F,
        'g' => GKey::G,
        'h' => GKey::H,
        'i' => GKey::I,
        'j' => GKey::J,
        'k' => GKey::K,
        'l' => GKey::L,
        'm' => GKey::M,
        'n' => GKey::N,
        'o' => GKey::O,
        'p' => GKey::P,
        'q' => GKey::Q,
        'r' => GKey::R,
        's' => GKey::S,
        't' => GKey::T,
        'u' => GKey::U,
        'v' => GKey::V,
        'w' => GKey::W,
        'x' => GKey::X,
        'y' => GKey::Y,
        'z' => GKey::Z,
        '0' => GKey::Digit0,
        '1' => GKey::Digit1,
        '2' => GKey::Digit2,
        '3' => GKey::Digit3,
        '4' => GKey::Digit4,
        '5' => GKey::Digit5,
        '6' => GKey::Digit6,
        '7' => GKey::Digit7,
        '8' => GKey::Digit8,
        '9' => GKey::Digit9,
        '`' => GKey::Backquote,
        '\\' => GKey::Backslash,
        '[' => GKey::BracketLeft,
        ']' => GKey::BracketRight,
        ',' => GKey::Comma,
        '=' => GKey::Equal,
        '-' => GKey::Minus,
        '.' => GKey::Period,
        '\'' => GKey::Quote,
        ';' => GKey::Semicolon,
        '/' => GKey::Slash,
        ' ' => GKey::Space,
        _ => return None,
    })
}

fn named(n: Named) -> GKey {
    match n {
        Named::Enter => GKey::Enter,
        Named::Tab => GKey::Tab,
        Named::Escape => GKey::Escape,
        Named::Backspace => GKey::Backspace,
        Named::Up => GKey::ArrowUp,
        Named::Down => GKey::ArrowDown,
        Named::Left => GKey::ArrowLeft,
        Named::Right => GKey::ArrowRight,
        Named::Home => GKey::Home,
        Named::End => GKey::End,
        Named::PageUp => GKey::PageUp,
        Named::PageDown => GKey::PageDown,
        Named::Insert => GKey::Insert,
        Named::Delete => GKey::Delete,
        Named::F(n) => match n {
            1 => GKey::F1,
            2 => GKey::F2,
            3 => GKey::F3,
            4 => GKey::F4,
            5 => GKey::F5,
            6 => GKey::F6,
            7 => GKey::F7,
            8 => GKey::F8,
            9 => GKey::F9,
            10 => GKey::F10,
            11 => GKey::F11,
            _ => GKey::F12,
        },
    }
}

/// The bytes that make `key` for a program in the state `modes`. Empty when the key makes
/// nothing (a bare modifier, say).
pub fn encode(mut key: Key, modes: PaneKeyModes) -> Vec<u8> {
    // Without the kitty protocol, Ctrl+i, Ctrl+m and Ctrl+[ are the bytes of Tab, Enter and
    // Esc. The encoder leaves them out, since those keys are told apart from them.
    if modes.kitty_flags == 0
        && key.mods.ctrl
        && let Code::Char(c) = key.code
        && let Some(n) = match c {
            'i' => Some(Named::Tab),
            'm' => Some(Named::Enter),
            '[' => Some(Named::Escape),
            _ => None,
        }
    {
        key.code = Code::Named(n);
        key.mods.ctrl = false;
    }
    let Ok(mut encoder) = Encoder::new() else {
        return Vec::new();
    };
    let Ok(mut event) = Event::new() else {
        return Vec::new();
    };
    let mut mods = GMods::empty();
    if key.mods.shift {
        mods |= GMods::SHIFT;
    }
    if key.mods.alt {
        mods |= GMods::ALT;
    }
    if key.mods.ctrl {
        mods |= GMods::CTRL;
    }
    if key.mods.super_key {
        mods |= GMods::SUPER;
    }
    event.set_mods(mods);
    match key.code {
        Code::Named(n) => {
            event.set_key(named(n));
        }
        Code::Char(c) => {
            event.set_key(physical(c).unwrap_or(GKey::Unidentified));
            event.set_unshifted_codepoint(c);
            // The text the key makes; with Ctrl the encoder works from the key, not the text.
            if !key.mods.ctrl {
                let shown = if key.mods.shift {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                };
                event.set_utf8(Some(shown.to_string()));
            }
        }
    }
    encoder.set_cursor_key_application(modes.app_cursor_keys);
    // Alt reaches a pane as an ESC prefix (the terminal mode default is on).
    encoder.set_alt_esc_prefix(true);
    encoder.set_macos_option_as_alt(OptionAsAlt::True);
    encoder.set_kitty_flags(KittyKeyFlags::from_bits_truncate(modes.kitty_flags));
    let mut out = Vec::new();
    let _ = encoder.encode_to_vec(&event, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: Code, mods: Mods) -> Key {
        Key { code, mods }
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            ..Mods::NONE
        }
    }

    #[test]
    fn plain_keys_make_their_legacy_bytes() {
        let m = PaneKeyModes::default();
        assert_eq!(encode(key(Code::Char('a'), Mods::NONE), m), b"a");
        assert_eq!(encode(key(Code::Named(Named::Enter), Mods::NONE), m), b"\r");
        assert_eq!(encode(key(Code::Named(Named::Tab), Mods::NONE), m), b"\t");
        assert_eq!(
            encode(key(Code::Named(Named::Escape), Mods::NONE), m),
            b"\x1b"
        );
        assert_eq!(encode(key(Code::Char('c'), ctrl()), m), b"\x03");
        assert_eq!(encode(key(Code::Char('m'), ctrl()), m), b"\r");
        assert_eq!(encode(key(Code::Char('i'), ctrl()), m), b"\t");
        assert_eq!(encode(key(Code::Char('['), ctrl()), m), b"\x1b");
        assert_eq!(
            encode(key(Code::Named(Named::Up), Mods::NONE), m),
            b"\x1b[A"
        );
        assert_eq!(
            encode(key(Code::Named(Named::Delete), Mods::NONE), m),
            b"\x1b[3~"
        );
        assert_eq!(
            encode(
                key(
                    Code::Char('x'),
                    Mods {
                        alt: true,
                        ..Mods::NONE
                    }
                ),
                m
            ),
            b"\x1bx"
        );
    }

    #[test]
    fn application_cursor_keys_change_the_arrows() {
        let m = PaneKeyModes {
            app_cursor_keys: true,
            kitty_flags: 0,
        };
        assert_eq!(
            encode(key(Code::Named(Named::Up), Mods::NONE), m),
            b"\x1bOA"
        );
    }

    #[test]
    fn the_kitty_protocol_tells_ctrl_m_from_enter() {
        let m = PaneKeyModes {
            app_cursor_keys: false,
            kitty_flags: 1,
        };
        assert_eq!(encode(key(Code::Char('m'), ctrl()), m), b"\x1b[109;5u");
        assert_eq!(encode(key(Code::Named(Named::Enter), Mods::NONE), m), b"\r");
    }
}
