/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The demo's byte codec (D41a). It is not a model tokenizer, and the UI labels it as demo.
//!
//! Ids 0 to 255 are the bytes of UTF-8 text. Id 256 is end-of-sequence and id 257 is padding.
//! Ids from 258 up are outside the vocabulary and are skipped when decoding.

use ncm::backend::text_codec_t;

/// The end-of-sequence id.
pub const EosId: u32 = 256;
/// The padding id. Decoding skips it.
pub const PadId: u32 = 257;
/// Ids 0 to 255 are bytes, then EOS and PAD.
pub const VocabSize: u32 = 258;

/// Maps text to its UTF-8 bytes and back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct byte_codec_t;

impl text_codec_t for byte_codec_t {
    fn encode(&self, text: &str) -> Vec<u32> {
        text.bytes().map(u32::from).collect()
    }

    /// Stops at the first EOS. Skips PAD and every id outside the byte range. Bytes that do not
    /// form valid UTF-8 become U+FFFD, so a character cut in half never panics.
    fn decode(&self, ids: &[u32]) -> String {
        let mut bytes = Vec::with_capacity(ids.len());
        for &id in ids {
            if id == EosId {
                break;
            }
            if let Ok(byte) = u8::try_from(id) {
                bytes.push(byte);
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_is_utf8_bytes() {
        assert_eq!(byte_codec_t.encode("Hi"), vec![72, 105]);
        assert_eq!(byte_codec_t.encode("é"), vec![195, 169]);
        assert!(byte_codec_t.encode("").is_empty());
    }

    #[test]
    fn round_trip_keeps_text() {
        let text = "Plain ASCII and ünïcode ✓";
        let ids = byte_codec_t.encode(text);
        assert_eq!(byte_codec_t.decode(&ids), text);
    }

    #[test]
    fn decode_stops_at_eos() {
        let ids = [72, 105, EosId, 33];
        assert_eq!(byte_codec_t.decode(&ids), "Hi");
    }

    #[test]
    fn decode_skips_pad_and_out_of_vocabulary_ids() {
        let ids = [PadId, 72, 258, 300, 105, PadId];
        assert_eq!(byte_codec_t.decode(&ids), "Hi");
    }

    #[test]
    fn a_character_cut_in_half_becomes_replacement() {
        // The first byte of a two-byte character, with nothing after it.
        let ids = [195];
        assert_eq!(byte_codec_t.decode(&ids), "\u{FFFD}");
        // A valid character followed by a stray continuation byte.
        let ids = [195, 169, 169];
        assert_eq!(byte_codec_t.decode(&ids), "é\u{FFFD}");
    }

    #[test]
    fn decode_of_nothing_is_empty() {
        assert_eq!(byte_codec_t.decode(&[]), "");
        assert_eq!(byte_codec_t.decode(&[EosId, 72]), "");
    }

    #[test]
    fn vocabulary_constants_agree() {
        assert_eq!(EosId, 256);
        assert_eq!(PadId, 257);
        assert_eq!(VocabSize, 258);
    }
}
