/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Copy through the terminal with the OSC 52 sequence (D48). The app queues the bytes and the
//! loop writes them through the terminal writer, so nothing deep in the code touches stdout.

/// Longest text sent, in bytes. Many terminals drop larger OSC 52 payloads.
pub const MaxCopy: usize = 100 * 1024;

const Alphabet: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk.first().copied().unwrap_or(0);
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        let sym = |shift: u32| Alphabet[((n >> shift) & 63) as usize] as char;
        out.push(sym(18));
        out.push(sym(12));
        out.push(if chunk.len() > 1 { sym(6) } else { '=' });
        out.push(if chunk.len() > 2 { sym(0) } else { '=' });
    }
    out
}

/// The OSC 52 sequence that puts `text` on the clipboard. Text past [`MaxCopy`] is cut at a
/// character boundary.
pub fn osc52(text: &str) -> Vec<u8> {
    let mut end = text.len().min(MaxCopy);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let payload = base64(text.get(..end).unwrap_or("").as_bytes());
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(b"\x1b]52;c;");
    out.extend_from_slice(payload.as_bytes());
    out.push(0x07);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn osc52_frame() {
        assert_eq!(osc52("hi"), b"\x1b]52;c;aGk=\x07".to_vec());
        let big = "é".repeat(MaxCopy);
        let seq = osc52(&big);
        assert!(seq.len() < MaxCopy * 2);
        assert_eq!(seq.last(), Some(&0x07));
    }
}
