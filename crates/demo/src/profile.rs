/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The two demo profiles (PROJECT.md, "UI prototype (P1)", "Demo backend").
//!
//! The numbers are demo values. They do not describe any released checkpoint, and the UI must
//! cope with both profiles.

use ncm::backend::{capabilities_t, sampler_t};

use crate::codec::{EosId, PadId, VocabSize};

/// Which demo profile a backend simulates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileKind {
    Standard,
    Compact,
}

impl ProfileKind {
    /// The name used in the command line and in the identity.
    pub fn name(self) -> &'static str {
        match self {
            ProfileKind::Standard => "standard",
            ProfileKind::Compact => "compact",
        }
    }

    /// The profile with this exact name, if there is one.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "standard" => Some(ProfileKind::Standard),
            "compact" => Some(ProfileKind::Compact),
            _ => None,
        }
    }
}

/// The limits and options of a profile.
///
/// Standard: P 4096, L 1024, K 5, level lengths 8, 32, 128, 512, 1024, with drafts.
/// Compact: P 1024, L 256, K 4, level lengths 16, 64, 128, 256, without drafts.
pub fn capabilities(kind: ProfileKind) -> capabilities_t {
    let (prefix_window, reply_len, level_lens, supports_drafts) = match kind {
        ProfileKind::Standard => (4096, 1024, vec![8, 32, 128, 512, 1024], true),
        ProfileKind::Compact => (1024, 256, vec![16, 64, 128, 256], false),
    };
    let level_count = level_lens.len() as u32;
    capabilities_t {
        prefix_window,
        reply_len,
        level_count,
        level_lens,
        eos_id: Some(EosId),
        pad_id: PadId,
        vocab_size: VocabSize,
        cut_at_eos: true,
        supports_drafts,
        default_sampler: sampler_t::greedy(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_profiles_validate() {
        assert_eq!(capabilities(ProfileKind::Standard).validate(), Ok(()));
        assert_eq!(capabilities(ProfileKind::Compact).validate(), Ok(()));
    }

    #[test]
    fn standard_profile_numbers() {
        let caps = capabilities(ProfileKind::Standard);
        assert_eq!(caps.prefix_window, 4096);
        assert_eq!(caps.reply_len, 1024);
        assert_eq!(caps.level_count, 5);
        assert_eq!(caps.level_lens, vec![8, 32, 128, 512, 1024]);
        assert!(caps.supports_drafts);
    }

    #[test]
    fn compact_profile_numbers() {
        let caps = capabilities(ProfileKind::Compact);
        assert_eq!(caps.prefix_window, 1024);
        assert_eq!(caps.reply_len, 256);
        assert_eq!(caps.level_count, 4);
        assert_eq!(caps.level_lens, vec![16, 64, 128, 256]);
        assert!(!caps.supports_drafts);
    }

    #[test]
    fn shared_fields_are_the_same_in_both_profiles() {
        for kind in [ProfileKind::Standard, ProfileKind::Compact] {
            let caps = capabilities(kind);
            assert_eq!(caps.eos_id, Some(256));
            assert_eq!(caps.pad_id, 257);
            assert_eq!(caps.vocab_size, 258);
            assert!(caps.cut_at_eos);
            assert_eq!(caps.default_sampler, sampler_t::greedy());
        }
    }

    #[test]
    fn names_parse_back() {
        for kind in [ProfileKind::Standard, ProfileKind::Compact] {
            assert_eq!(ProfileKind::parse(kind.name()), Some(kind));
        }
        assert_eq!(ProfileKind::parse("Standard"), None);
        assert_eq!(ProfileKind::parse(""), None);
    }
}
