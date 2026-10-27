/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The demo backend: `backend_t` over scripted replies (PROJECT.md, "Backend contract (P1)" and
//! "Demo backend").
//!
//! A call runs `K` level passes in order. Each pass sleeps for the chosen speed, then reports a
//! pseudo-random code block and, for a profile with drafts, a draft of the reply. After the last
//! pass the reply is returned as `L` ids. None of this is model output (O1).

use std::time::Instant;

use ncm::backend::{
    BackendError, BackendKind, CallOutcome, backend_t, call_request_t, cancelled_t, capabilities_t,
    completion_t, level_progress_t, model_identity_t, text_codec_t,
};

use crate::codec::{EosId, PadId, byte_codec_t};
use crate::profile::{ProfileKind, capabilities};
use crate::scenario::{ScriptedOutcome, scripted_reply};
use crate::timing::{sleep_checked, timing_t};

/// The one-line note in the identity of every demo backend.
const DemoNote: &str = "DEMO: scripted simulator, not an NCM model";

/// The increment of the splitmix64 generator.
const GoldenGamma: u64 = 0x9E37_79B9_7F4A_7C15;

/// Codes are drawn below this bound.
const CodeBound: u64 = 1024;

/// A simulated backend that answers with scripted replies.
pub struct demo_backend_t {
    timing: timing_t,
    identity: model_identity_t,
    caps: capabilities_t,
    codec: byte_codec_t,
}

impl demo_backend_t {
    /// A backend for one profile, with the given simulated delays.
    pub fn new(profile: ProfileKind, timing: timing_t) -> Self {
        demo_backend_t {
            timing,
            identity: model_identity_t {
                id: format!("demo-{}", profile.name()),
                kind: BackendKind::Demo,
                export_hash: None,
                note: DemoNote.to_string(),
            },
            caps: capabilities(profile),
            codec: byte_codec_t,
        }
    }
}

impl backend_t for demo_backend_t {
    fn identity(&self) -> &model_identity_t {
        &self.identity
    }

    fn capabilities(&self) -> &capabilities_t {
        &self.caps
    }

    fn codec(&self) -> &dyn text_codec_t {
        &self.codec
    }

    fn run_call(
        &self,
        request: call_request_t,
        on_level: &mut dyn FnMut(level_progress_t),
    ) -> Result<CallOutcome, BackendError> {
        self.caps.validate()?;
        request.validate_for(&self.caps)?;

        let prefix_text = self.codec.decode(&request.prefix);
        let scripted = scripted_reply(&prefix_text, request.seed);
        let (reply_text, failure) = match scripted.outcome {
            ScriptedOutcome::Reply(text) => (text, None),
            ScriptedOutcome::FailAfterLevel { level, message } => {
                (String::new(), Some((level, message)))
            }
        };
        let reply_len = self.caps.reply_len as usize;
        let tokens = reply_tokens(&self.codec, &reply_text, reply_len);
        // The text the completion decodes to. Drafts converge on it.
        let final_text = self.codec.decode(&tokens);
        let level_count = self.caps.level_count as usize;

        for (level, level_len) in self.caps.level_lens.iter().copied().enumerate() {
            if request.cancel.is_cancelled() {
                return Ok(CallOutcome::Cancelled(cancelled_t {
                    levels_done: level as u32,
                }));
            }
            let started = Instant::now();
            // A cancel that arrives during the pass does not stop it. The check before the next
            // pass sees the token.
            sleep_checked(self.timing.level_ms, &request.cancel);
            let elapsed = started.elapsed();
            let draft = self
                .caps
                .supports_drafts
                .then(|| draft_for(&final_text, level, level_count));
            on_level(level_progress_t {
                level: level as u32,
                level_count: self.caps.level_count,
                level_len,
                codes: level_codes(request.seed, level as u32, level_len),
                elapsed,
                draft,
            });
            if let Some((fail_level, message)) = &failure
                && *fail_level as usize == level
            {
                return Err(BackendError::Failed {
                    message: message.clone(),
                });
            }
        }

        Ok(CallOutcome::Completed(completion_t { tokens }))
    }
}

/// The `L` ids of a reply: its bytes, then EOS when they leave room, then PAD up to `L`.
///
/// A reply of `L` bytes or more is cut at `L` with no EOS, which the harness reads as truncated.
fn reply_tokens(codec: &byte_codec_t, text: &str, reply_len: usize) -> Vec<u32> {
    let mut tokens: Vec<u32> = codec.encode(text).into_iter().take(reply_len).collect();
    if tokens.len() < reply_len {
        tokens.push(EosId);
    }
    tokens.resize(reply_len, PadId);
    tokens
}

/// The draft after level `level`. The last level shows the exact final text. Earlier levels show
/// the blurred text of [`blurred_draft`].
fn draft_for(final_text: &str, level: usize, level_count: usize) -> String {
    if level + 1 >= level_count {
        final_text.to_string()
    } else {
        blurred_draft(final_text, level, level_count)
    }
}

/// The text after level `level` of `level_count`, blurred so that fewer words show early.
///
/// The words are merged into `ceil(words * (level + 1) / level_count)` groups. Each group is its
/// words joined with no space, and the groups are joined with single spaces. So each token of the
/// draft is one block, and its width is the group's length. The result has no words after the
/// last level.
fn blurred_draft(text: &str, level: usize, level_count: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let total = words.len();
    // `level_count` is at least 2 here, because the last level is never blurred.
    let groups = (total * (level + 1)).div_ceil(level_count).min(total);
    (0..groups)
        .map(|group| {
            let start = group * total / groups;
            let end = (group + 1) * total / groups;
            words.get(start..end).unwrap_or(&[]).concat()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// Codes for one level: `level_len` values below 1024, from splitmix64 of (seed, level, index).
fn level_codes(seed: u64, level: u32, level_len: u32) -> Vec<u32> {
    let base = splitmix64(seed ^ splitmix64(u64::from(level)));
    (0..level_len)
        .map(|index| (splitmix64(base ^ u64::from(index)) % CodeBound) as u32)
        .collect()
}

fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(GoldenGamma);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ncm::backend::{cancel_token_t, sampler_t};

    fn backend(profile: ProfileKind) -> demo_backend_t {
        demo_backend_t::new(profile, timing_t::instant())
    }

    fn request(prefix: Vec<u32>, seed: u64, cancel: &cancel_token_t) -> call_request_t {
        call_request_t {
            prefix,
            sampler: sampler_t::greedy(),
            seed,
            cancel: cancel.clone(),
        }
    }

    fn prefix_for(user: &str) -> Vec<u32> {
        byte_codec_t.encode(&format!("User: {user}\nAssistant:"))
    }

    /// The text a scenario replies with at step 0.
    fn scripted_text(user: &str, seed: u64) -> String {
        match scripted_reply(&format!("User: {user}\nAssistant:"), seed).outcome {
            ScriptedOutcome::Reply(text) => text,
            ScriptedOutcome::FailAfterLevel { .. } => String::new(),
        }
    }

    fn collect(
        backend: &demo_backend_t,
        request: call_request_t,
    ) -> (Result<CallOutcome, BackendError>, Vec<level_progress_t>) {
        let mut seen = Vec::new();
        let result = backend.run_call(request, &mut |progress| seen.push(progress));
        (result, seen)
    }

    fn completed_tokens(result: Result<CallOutcome, BackendError>) -> Vec<u32> {
        match result {
            Ok(CallOutcome::Completed(done)) => done.tokens,
            other => panic!("expected a completed call, got {other:?}"),
        }
    }

    #[test]
    fn identity_names_the_demo_and_no_model() {
        let standard = backend(ProfileKind::Standard);
        let identity = standard.identity();
        assert_eq!(identity.id, "demo-standard");
        assert_eq!(identity.kind, BackendKind::Demo);
        assert_eq!(identity.export_hash, None);
        assert_eq!(identity.note, DemoNote);
        assert!(identity.simulated());

        assert_eq!(backend(ProfileKind::Compact).identity().id, "demo-compact");
    }

    #[test]
    fn capabilities_come_from_the_profile() {
        let standard = backend(ProfileKind::Standard);
        assert_eq!(
            standard.capabilities(),
            &capabilities(ProfileKind::Standard)
        );
        let compact = backend(ProfileKind::Compact);
        assert_eq!(compact.capabilities(), &capabilities(ProfileKind::Compact));
    }

    #[test]
    fn standard_call_reports_each_level_with_drafts_and_completes() {
        let demo = backend(ProfileKind::Standard);
        let cancel = cancel_token_t::new();
        let (result, seen) = collect(&demo, request(prefix_for("hello"), 4242, &cancel));
        let tokens = completed_tokens(result);

        assert_eq!(tokens.len(), 1024);
        let reply = scripted_text("hello", 4242);
        let eos = tokens.iter().position(|id| *id == EosId);
        assert_eq!(eos, Some(reply.len()));
        assert!(tokens[reply.len() + 1..].iter().all(|id| *id == PadId));
        assert_eq!(codec_text(&tokens), reply);

        assert_eq!(seen.len(), 5);
        let lens: Vec<u32> = seen.iter().map(|p| p.level_len).collect();
        assert_eq!(lens, vec![8, 32, 128, 512, 1024]);
        for (index, progress) in seen.iter().enumerate() {
            assert_eq!(progress.level, index as u32);
            assert_eq!(progress.level_count, 5);
            assert_eq!(progress.codes.len(), progress.level_len as usize);
            assert!(progress.codes.iter().all(|code| *code < 1024));
            assert!(progress.draft.is_some());
        }

        // The first draft is blurred into one group, and the last draft is the exact text.
        let words = reply.split_whitespace().count();
        let first = seen[0].draft.clone().unwrap_or_default();
        assert_eq!(first.split_whitespace().count(), words.div_ceil(5));
        assert_eq!(seen[4].draft.as_deref(), Some(codec_text(&tokens).as_str()));
    }

    fn codec_text(tokens: &[u32]) -> String {
        byte_codec_t.decode(tokens)
    }

    #[test]
    fn compact_call_has_four_levels_and_no_drafts() {
        let demo = backend(ProfileKind::Compact);
        let cancel = cancel_token_t::new();
        let (result, seen) = collect(&demo, request(prefix_for("hello"), 4242, &cancel));
        let tokens = completed_tokens(result);

        assert_eq!(tokens.len(), 256);
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().all(|progress| progress.draft.is_none()));
        let lens: Vec<u32> = seen.iter().map(|p| p.level_len).collect();
        assert_eq!(lens, vec![16, 64, 128, 256]);
    }

    #[test]
    fn long_reply_is_cut_under_compact_and_whole_under_standard() {
        let cancel = cancel_token_t::new();
        let text = scripted_text("long", 4242);
        assert!(text.len() > 256 && text.len() < 1024);

        let compact = backend(ProfileKind::Compact);
        let tokens =
            completed_tokens(collect(&compact, request(prefix_for("long"), 4242, &cancel)).0);
        assert_eq!(tokens.len(), 256);
        assert!(!tokens.contains(&EosId), "a cut reply has no EOS");
        assert_eq!(codec_text(&tokens), text[..256]);

        let standard = backend(ProfileKind::Standard);
        let tokens =
            completed_tokens(collect(&standard, request(prefix_for("long"), 4242, &cancel)).0);
        assert_eq!(tokens.len(), 1024);
        assert!(tokens.contains(&EosId));
        assert_eq!(codec_text(&tokens), text);
    }

    #[test]
    fn reply_tokens_pad_short_replies_and_cut_long_ones() {
        let codec = byte_codec_t;
        assert_eq!(
            reply_tokens(&codec, "ab", 5),
            vec![97, 98, EosId, PadId, PadId]
        );
        let exact = "a".repeat(5);
        assert_eq!(reply_tokens(&codec, &exact, 5), vec![97; 5]);
        let longer = "a".repeat(9);
        assert_eq!(reply_tokens(&codec, &longer, 5), vec![97; 5]);
        assert_eq!(reply_tokens(&codec, "", 3), vec![EosId, PadId, PadId]);
    }

    #[test]
    fn a_token_set_before_level_zero_cancels_with_no_levels() {
        let demo = backend(ProfileKind::Standard);
        let cancel = cancel_token_t::new();
        cancel.cancel();
        let (result, seen) = collect(&demo, request(prefix_for("hello"), 1, &cancel));
        assert_eq!(
            result,
            Ok(CallOutcome::Cancelled(cancelled_t { levels_done: 0 }))
        );
        assert!(seen.is_empty());
    }

    #[test]
    fn a_cancel_during_a_pass_stops_before_the_next_pass() {
        let demo = backend(ProfileKind::Standard);
        let cancel = cancel_token_t::new();
        let flag = cancel.clone();
        let mut seen = Vec::new();
        let result = demo.run_call(request(prefix_for("hello"), 1, &cancel), &mut |progress| {
            if progress.level == 1 {
                flag.cancel();
            }
            seen.push(progress.level);
        });
        assert_eq!(
            result,
            Ok(CallOutcome::Cancelled(cancelled_t { levels_done: 2 }))
        );
        assert_eq!(seen, vec![0, 1]);
    }

    #[test]
    fn the_error_scenario_fails_after_level_one() {
        let demo = backend(ProfileKind::Standard);
        let cancel = cancel_token_t::new();
        let (result, seen) = collect(&demo, request(prefix_for("error"), 1, &cancel));
        assert_eq!(
            result,
            Err(BackendError::Failed {
                message: "simulated backend failure: scripted scenario `error`".to_string(),
            })
        );
        let levels: Vec<u32> = seen.iter().map(|p| p.level).collect();
        assert_eq!(levels, vec![0, 1]);
    }

    #[test]
    fn invalid_requests_are_refused_before_any_pass() {
        let demo = backend(ProfileKind::Compact);
        let cancel = cancel_token_t::new();

        let (empty, seen) = collect(&demo, request(Vec::new(), 1, &cancel));
        assert!(matches!(empty, Err(BackendError::InvalidRequest { .. })));

        let (long, _) = collect(&demo, request(vec![0; 1025], 1, &cancel));
        assert_eq!(
            long.err(),
            Some(BackendError::PrefixTooLong {
                len: 1025,
                window: 1024
            })
        );

        let (outside, _) = collect(&demo, request(vec![300], 1, &cancel));
        assert!(matches!(outside, Err(BackendError::InvalidRequest { .. })));
        assert!(seen.is_empty());
    }

    #[test]
    fn a_standard_window_accepts_its_full_prefix() {
        let demo = backend(ProfileKind::Standard);
        let cancel = cancel_token_t::new();
        let (result, _) = collect(&demo, request(vec![72; 4096], 1, &cancel));
        assert!(result.is_ok());
    }

    #[test]
    fn flaky_even_seed_returns_the_invalid_sample_as_its_completion() {
        let demo = backend(ProfileKind::Standard);
        let cancel = cancel_token_t::new();
        let (even, _) = collect(&demo, request(prefix_for("flaky"), 4242, &cancel));
        assert!(codec_text(&completed_tokens(even)).contains("@call no_such_tool"));
        let (odd, _) = collect(&demo, request(prefix_for("flaky"), 4243, &cancel));
        assert!(!codec_text(&completed_tokens(odd)).contains("no_such_tool"));
    }

    #[test]
    fn blurred_draft_merges_words_into_growing_groups() {
        let text = "one two three four";
        assert_eq!(blurred_draft(text, 0, 5), "onetwothreefour");
        assert_eq!(blurred_draft(text, 1, 5), "onetwo threefour");
        assert_eq!(blurred_draft(text, 3, 5), "one two three four");
        assert_eq!(blurred_draft("", 0, 5), "");
    }

    #[test]
    fn draft_for_shows_the_exact_text_on_the_last_level() {
        assert_eq!(draft_for("a b c", 4, 5), "a b c");
        assert_eq!(draft_for("a b c", 0, 5), "abc");
    }

    #[test]
    fn level_codes_are_deterministic_bounded_and_seed_dependent() {
        let first = level_codes(7, 2, 128);
        assert_eq!(first, level_codes(7, 2, 128));
        assert_eq!(first.len(), 128);
        assert!(first.iter().all(|code| *code < 1024));
        assert_ne!(first, level_codes(8, 2, 128));
        assert_ne!(first, level_codes(7, 3, 128));
        assert!(level_codes(7, 0, 0).is_empty());
    }
}
