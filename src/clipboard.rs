//! Copying to the system clipboard through the terminal (OSC 52). The terminal
//! does the work, so it also works over SSH. Terminals that do not support it
//! ignore the sequence.

/// Longest text sent. Many terminals drop larger sequences.
const MAX_BYTES: usize = 75_000;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
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

/// The escape sequence that puts `text` on the system clipboard, or `None`
/// when there is nothing to copy or it is too long.
pub fn osc52(text: &str) -> Option<String> {
    if text.is_empty() || text.len() > MAX_BYTES {
        return None;
    }
    Some(format!("\x1b]52;c;{}\x07", base64(text.as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_examples() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64("é".as_bytes()), "w6k=");
    }

    #[test]
    fn the_sequence_carries_the_text_and_refuses_nothing_or_too_much() {
        assert_eq!(osc52("foo").as_deref(), Some("\x1b]52;c;Zm9v\x07"));
        assert_eq!(osc52(""), None);
        assert_eq!(osc52(&"x".repeat(MAX_BYTES + 1)), None);
    }
}
