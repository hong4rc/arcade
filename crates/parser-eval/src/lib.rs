//! Evaluation of escape sequence parser candidates for Arcade.
//!
//! Run with `cargo parser-eval all`. See `docs/decisions/001-escape-sequence-parser.md`.

pub mod cand_alacritty;
pub mod cand_ghostty;
pub mod cand_vt100;
pub mod cand_vte_min;
pub mod candidate;
pub mod cases;
pub mod procstat;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Strict decoder: returns `None` for anything that is not valid base64.
pub fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let trimmed: &[u8] = match text {
        [rest @ .., b'=', b'='] => rest,
        [rest @ .., b'='] => rest,
        all => all,
    };
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() / 4 * 3 + 2);
    for chunk in trimmed.chunks(4) {
        let mut n = 0u32;
        for (i, &byte) in chunk.iter().enumerate() {
            let value = ALPHABET.iter().position(|&a| a == byte)? as u32;
            n |= value << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_every_length() {
        for len in 0..64usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let encoded = base64_encode(&data);
            assert_eq!(
                base64_decode(encoded.as_bytes()),
                Some(data),
                "length {len}"
            );
        }
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_decode(b"aGVsbG8="), Some(b"hello".to_vec()));
    }

    #[test]
    fn base64_rejects_invalid_input() {
        assert_eq!(base64_decode(b"!!!not*base64!!!"), None);
        assert_eq!(base64_decode(b"abcde"), None);
    }
}
