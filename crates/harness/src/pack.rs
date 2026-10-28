/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Prototype prompt packing: the plain template of D14, fitted to the window P (D43).
//!
//! Whole entries are dropped from the oldest end until the encoded prefix fits in `P`. When even
//! the newest entry does not fit, its tail is kept. The packer never returns more than `P` ids.
//! It returns no ids only when `P` is 0, which `capabilities_t::validate` rejects; a caller must
//! validate the capabilities first.

use ncm::backend::{capabilities_t, text_codec_t};

/// The open line that ends every prefix. It has no trailing newline.
const OpenLine: &str = "Assistant:";

const FnvOffsetBasis: u64 = 0xcbf2_9ce4_8422_2325;
const FnvPrime: u64 = 0x0000_0100_0000_01b3;

/// One entry of the transcript, as the packer sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptEntry {
    User { text: String },
    Assistant { text: String },
    ToolResult { tool: String, text: String },
    ToolError { tool: String, message: String },
    UserAnswer { text: String },
}

/// Renders one entry in the plain template, with its trailing newline.
pub fn render_entry(entry: &TranscriptEntry) -> String {
    match entry {
        TranscriptEntry::User { text } => format!("User: {text}\n"),
        TranscriptEntry::Assistant { text } => format!("Assistant: {text}\n"),
        TranscriptEntry::ToolResult { tool, text } => format!("Tool {tool}: {text}\n"),
        TranscriptEntry::ToolError { tool, message } => format!("Tool error {tool}: {message}\n"),
        TranscriptEntry::UserAnswer { text } => format!("User answer: {text}\n"),
    }
}

/// A packed prefix and what was left out of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct packed_prefix_t {
    /// At most `P` ids, never more.
    pub ids: Vec<u32>,
    /// Whole entries dropped from the oldest end. With a cut newest entry, every older entry is
    /// counted here.
    pub dropped_entries: usize,
    /// FNV-1a 64 of the ids, as 16 lowercase hex characters (see `ids_hash`).
    pub hash: String,
}

/// Packs the transcript into a prefix of at most `caps.prefix_window` ids.
pub fn pack_prefix(
    codec: &dyn text_codec_t,
    caps: &capabilities_t,
    entries: &[TranscriptEntry],
) -> packed_prefix_t {
    let window = caps.prefix_window as usize;
    for start in 0..entries.len() {
        let ids = codec.encode(&prompt_from(entries, start));
        if ids.len() <= window {
            return packed(ids, start);
        }
    }
    let newest = entries.len().saturating_sub(1);
    let mut ids = codec.encode(&prompt_from(entries, newest));
    if ids.len() > window {
        let cut = ids.len() - window;
        ids.drain(..cut);
    }
    packed(ids, newest)
}

/// The text of the entries from `start` on, then the open line.
fn prompt_from(entries: &[TranscriptEntry], start: usize) -> String {
    let mut text: String = entries.iter().skip(start).map(render_entry).collect();
    text.push_str(OpenLine);
    text
}

fn packed(ids: Vec<u32>, dropped_entries: usize) -> packed_prefix_t {
    let hash = ids_hash(&ids);
    packed_prefix_t {
        ids,
        dropped_entries,
        hash,
    }
}

/// FNV-1a 64 over the bytes. The offset basis and prime are the standard 64-bit values.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FnvOffsetBasis, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FnvPrime)
    })
}

/// FNV-1a 64 over the ids as little-endian `u32` bytes, as 16 lowercase hex characters.
pub fn ids_hash(ids: &[u32]) -> String {
    let bytes: Vec<u8> = ids.iter().flat_map(|id| id.to_le_bytes()).collect();
    let hash = fnv1a64(&bytes);
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ncm::backend::sampler_t;

    /// A codec whose id is the byte value.
    struct byte_codec_t;

    impl text_codec_t for byte_codec_t {
        fn encode(&self, text: &str) -> Vec<u32> {
            text.bytes().map(u32::from).collect()
        }

        fn decode(&self, ids: &[u32]) -> String {
            let bytes: Vec<u8> = ids.iter().filter_map(|id| u8::try_from(*id).ok()).collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
    }

    fn caps(prefix_window: u32) -> capabilities_t {
        capabilities_t {
            prefix_window,
            reply_len: 1,
            level_count: 1,
            level_lens: vec![1],
            eos_id: None,
            pad_id: 0,
            vocab_size: 256,
            cut_at_eos: false,
            supports_drafts: false,
            default_sampler: sampler_t::greedy(),
        }
    }

    fn user(text: &str) -> TranscriptEntry {
        TranscriptEntry::User {
            text: text.to_string(),
        }
    }

    fn decoded(ids: &[u32]) -> String {
        byte_codec_t.decode(ids)
    }

    #[test]
    fn each_entry_renders_in_the_plain_template() {
        assert_eq!(render_entry(&user("hi")), "User: hi\n");
        assert_eq!(
            render_entry(&TranscriptEntry::Assistant {
                text: "ok".to_string()
            }),
            "Assistant: ok\n"
        );
        assert_eq!(
            render_entry(&TranscriptEntry::ToolResult {
                tool: "list_dir".to_string(),
                text: "a b".to_string()
            }),
            "Tool list_dir: a b\n"
        );
        assert_eq!(
            render_entry(&TranscriptEntry::ToolError {
                tool: "read_file".to_string(),
                message: "no such file".to_string()
            }),
            "Tool error read_file: no such file\n"
        );
        assert_eq!(
            render_entry(&TranscriptEntry::UserAnswer {
                text: "yes".to_string()
            }),
            "User answer: yes\n"
        );
    }

    #[test]
    fn an_empty_transcript_packs_the_open_line_only() {
        let packed = pack_prefix(&byte_codec_t, &caps(64), &[]);
        assert_eq!(decoded(&packed.ids), "Assistant:");
        assert_eq!(packed.dropped_entries, 0);
    }

    #[test]
    fn the_full_transcript_is_kept_when_it_fits() {
        // "User: aaaa\n" and "User: bbbb\n" are 11 bytes each, the open line is 10.
        let entries = vec![user("aaaa"), user("bbbb")];
        let packed = pack_prefix(&byte_codec_t, &caps(32), &entries);
        assert_eq!(decoded(&packed.ids), "User: aaaa\nUser: bbbb\nAssistant:");
        assert_eq!(packed.dropped_entries, 0);
    }

    #[test]
    fn whole_entries_are_dropped_from_the_oldest_end() {
        // "User: aaaa\n" is 11 bytes, "Assistant: bbbb\n" 16, "User: cc\n" 9 and the open line 10.
        let entries = vec![
            user("aaaa"),
            TranscriptEntry::Assistant {
                text: "bbbb".to_string(),
            },
            user("cc"),
        ];
        let kept_two = pack_prefix(&byte_codec_t, &caps(35), &entries);
        assert_eq!(kept_two.dropped_entries, 1);
        assert_eq!(
            decoded(&kept_two.ids),
            "Assistant: bbbb\nUser: cc\nAssistant:"
        );
        let kept_one = pack_prefix(&byte_codec_t, &caps(20), &entries);
        assert_eq!(kept_one.dropped_entries, 2);
        assert_eq!(decoded(&kept_one.ids), "User: cc\nAssistant:");
    }

    #[test]
    fn a_newest_entry_that_is_too_long_keeps_its_tail() {
        let entries = vec![user("hello world")];
        let packed = pack_prefix(&byte_codec_t, &caps(5), &entries);
        assert_eq!(packed.ids.len(), 5);
        assert_eq!(decoded(&packed.ids), "tant:");
        assert_eq!(packed.dropped_entries, 0);
    }

    #[test]
    fn the_tail_case_counts_every_older_entry_as_dropped() {
        let entries = vec![user("one"), user("two"), user("a long newest entry")];
        let packed = pack_prefix(&byte_codec_t, &caps(12), &entries);
        assert_eq!(packed.ids.len(), 12);
        assert_eq!(packed.dropped_entries, 2);
        assert_eq!(decoded(&packed.ids), "y\nAssistant:");
    }

    #[test]
    fn the_prefix_never_exceeds_the_window_and_is_never_empty() {
        let entries = vec![
            user("first turn with some text"),
            TranscriptEntry::ToolResult {
                tool: "read_file".to_string(),
                text: "line one\nline two".to_string(),
            },
            TranscriptEntry::UserAnswer {
                text: "x".repeat(40),
            },
        ];
        for window in 1..=200u32 {
            let packed = pack_prefix(&byte_codec_t, &caps(window), &entries);
            assert!(packed.ids.len() <= window as usize, "window {window}");
            assert!(!packed.ids.is_empty(), "window {window}");
            assert_eq!(packed.hash, ids_hash(&packed.ids));
        }
    }

    #[test]
    fn the_hash_is_fnv1a_over_little_endian_ids() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
        assert_eq!(ids_hash(&[]), "cbf29ce484222325");
        assert_eq!(ids_hash(&[1, 256, 65536]), "247342c414131b96");
    }

    #[test]
    fn the_packed_hash_is_16_lowercase_hex_characters() {
        let packed = pack_prefix(&byte_codec_t, &caps(64), &[user("hi")]);
        assert_eq!(packed.hash.len(), 16);
        assert!(
            packed
                .hash
                .chars()
                .all(|ch| matches!(ch, '0'..='9' | 'a'..='f'))
        );
        assert_eq!(packed.hash, ids_hash(&packed.ids));
    }
}
