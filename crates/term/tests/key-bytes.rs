//! A printable key, with or without Shift, never turns into anything but its own text or a
//! well-formed escape sequence, whatever the program in the pane asked for.

use arcade_term::{Code, Key, Mods, PaneKeyModes, encode_key as encode};

#[test]
fn a_printable_key_never_makes_a_stray_byte() {
    let mut bad = Vec::new();
    for flags in 0u8..=31 {
        for app_cursor_keys in [false, true] {
            for shift in [false, true] {
                for c in ('a'..='z').chain('0'..='9').chain([' ', '.', ',', '-']) {
                    let key = Key {
                        code: Code::Char(c),
                        mods: Mods {
                            shift,
                            ..Mods::NONE
                        },
                    };
                    let out = encode(
                        key,
                        PaneKeyModes {
                            app_cursor_keys,
                            kitty_flags: flags,
                        },
                    );
                    // Text, or an escape sequence made of printable bytes.
                    let fine = !out.is_empty()
                        && (out == c.to_string().as_bytes()
                            || out == c.to_ascii_uppercase().to_string().as_bytes()
                            || (out[0] == 0x1b
                                && out[1..].iter().all(|b| (0x20..0x7f).contains(b))));
                    if !fine {
                        bad.push((flags, shift, c, out));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} odd encodings, first: {:?}",
        bad.len(),
        &bad[..bad.len().min(5)]
    );
}

#[test]
fn the_same_key_encoded_many_times_gives_the_same_bytes() {
    let key = Key {
        code: Code::Char('e'),
        mods: Mods::NONE,
    };
    for flags in [0u8, 1, 3, 8, 11, 31] {
        let modes = PaneKeyModes {
            app_cursor_keys: false,
            kitty_flags: flags,
        };
        let first = encode(key, modes);
        for _ in 0..20_000 {
            assert_eq!(encode(key, modes), first, "flags {flags}");
        }
    }
}
