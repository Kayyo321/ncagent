/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! The simulated backend, simulated tools and demo state store (PROJECT.md, O1 and D36).
//!
//! Everything in this crate is a simulation. Nothing here runs a model, a shell command or a
//! network request, and nothing here edits a project file. Only the binary depends on it.

pub mod backend;
pub mod codec;
pub mod paths;
pub mod profile;
pub mod scenario;
pub mod store;
pub mod timing;
pub mod tools;

use std::sync::Arc;

use harness::api::hint_t;

use crate::backend::demo_backend_t;
use crate::profile::ProfileKind;
use crate::timing::timing_t;
use crate::tools::demo_tool_runner_t;

/// The label on every simulated session and record (O1 point 6).
pub const Label: &str = "DEMO: simulated model, not NCM output";

/// The line the UI shows on every screen in demo mode (O1 point 6).
pub const Notice: &str =
    "DEMO MODE: replies and tools are simulated. No model is running and nothing is executed.";

/// The trigger words of the scripted scenarios, each with what it shows.
pub fn hints() -> Vec<hint_t> {
    [
        (
            "tool",
            "Lists a folder, then reads a file. Read tools run without asking.",
        ),
        (
            "approve",
            "Asks to edit a file. Approving shows the simulated result.",
        ),
        (
            "deny",
            "Asks to edit a file. Denying shows the denial reply.",
        ),
        (
            "shell",
            "Asks to run an echo command. The output is simulated.",
        ),
        (
            "ask",
            "Shows a question with three options, then repeats your answer.",
        ),
        ("fail", "Reads a missing file, so the simulated tool fails."),
        (
            "error",
            "The backend fails after level 1 with a simulated error.",
        ),
        (
            "flaky",
            "Even seeds send an invalid reply first, then a valid one.",
        ),
        ("long", "A long reply, cut short under the compact profile."),
        (
            "markdown",
            "A Markdown showcase with a table and a code block.",
        ),
    ]
    .into_iter()
    .map(|(trigger, description)| hint_t {
        trigger: trigger.to_string(),
        description: description.to_string(),
    })
    .collect()
}

/// The simulated backend and the simulated tool runner, with the same delays.
pub fn build(
    profile: ProfileKind,
    timing: timing_t,
) -> (Arc<demo_backend_t>, Arc<demo_tool_runner_t>) {
    (
        Arc::new(demo_backend_t::new(profile, timing)),
        Arc::new(demo_tool_runner_t::new(timing)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::event::ReplyItem;
    use harness::reply::parse_reply;
    use harness::tools::{parsed_call_t, tool_call_t, tool_runner_t};
    use ncm::backend::{
        BackendError, CallOutcome, backend_t, call_request_t, cancel_token_t, level_progress_t,
        sampler_t, text_codec_t,
    };

    use crate::codec::byte_codec_t;

    fn prefix_of(lines: &[String]) -> Vec<u32> {
        let mut text = lines.join("\n");
        text.push_str("\nAssistant:");
        byte_codec_t.encode(&text)
    }

    /// Runs one call and returns the completed reply text, or the error.
    fn reply_for(
        demo: &demo_backend_t,
        lines: &[String],
        seed: u64,
    ) -> Result<String, BackendError> {
        let request = call_request_t {
            prefix: prefix_of(lines),
            sampler: sampler_t::greedy(),
            seed,
            cancel: cancel_token_t::new(),
        };
        match demo.run_call(request, &mut |_: level_progress_t| {})? {
            CallOutcome::Completed(done) => Ok(byte_codec_t.decode(&done.tokens)),
            CallOutcome::Cancelled(_) => panic!("the call was not cancelled"),
        }
    }

    fn calls_of(items: &[ReplyItem]) -> Vec<parsed_call_t> {
        items
            .iter()
            .filter_map(|item| match item {
                ReplyItem::Call { call } => Some(call.clone()),
                _ => None,
            })
            .collect()
    }

    fn standard() -> demo_backend_t {
        demo_backend_t::new(ProfileKind::Standard, timing_t::instant())
    }

    #[test]
    fn every_scenario_first_reply_parses_with_the_harness_parser() {
        let demo = standard();
        for trigger in [
            "tool", "approve", "deny", "shell", "ask", "fail", "markdown", "long",
        ] {
            let lines = vec![format!("User: {trigger}")];
            let text = reply_for(&demo, &lines, 4242).expect("the call completes");
            let parsed = parse_reply(&text);
            assert!(parsed.issues.is_empty(), "{trigger}: {:?}", parsed.issues);
            assert!(parsed.valid(), "{trigger} must parse to at least one item");
            if trigger == "ask" {
                let asks_three = parsed.items.iter().any(|item| {
                    matches!(item, ReplyItem::Ask { question } if question.options.len() == 3)
                });
                assert!(
                    asks_three,
                    "the ask scenario must parse to a three-option question"
                );
            }
        }
    }

    #[test]
    fn the_call_blocks_of_the_scripts_name_registered_tools() {
        let demo = standard();
        for trigger in ["tool", "approve", "shell", "fail"] {
            let lines = vec![format!("User: {trigger}")];
            let text = reply_for(&demo, &lines, 4242).expect("the call completes");
            let parsed = parse_reply(&text);
            let calls = calls_of(&parsed.items);
            assert_eq!(calls.len(), 1, "{trigger} asks for one tool");
            assert!(harness::tools::find(&calls[0].tool).is_some());
        }
    }

    #[test]
    fn flaky_is_invalid_on_an_even_seed_and_valid_on_an_odd_one() {
        let demo = standard();
        let lines = vec!["User: flaky".to_string()];
        let even = parse_reply(&reply_for(&demo, &lines, 4242).expect("completes"));
        assert!(!even.issues.is_empty(), "the even sample must be invalid");
        let odd = parse_reply(&reply_for(&demo, &lines, 4243).expect("completes"));
        assert!(odd.valid(), "{:?}", odd.issues);
    }

    #[test]
    fn the_error_scenario_surfaces_as_a_backend_failure() {
        let demo = standard();
        let lines = vec!["User: error".to_string()];
        assert_eq!(
            reply_for(&demo, &lines, 1),
            Err(BackendError::Failed {
                message: "simulated backend failure: scripted scenario `error`".to_string(),
            })
        );
    }

    #[test]
    fn the_tool_scenario_runs_end_to_end_through_codec_parser_and_runner() {
        let demo = standard();
        let runner = demo_tool_runner_t::new(timing_t::instant());
        let cancel = cancel_token_t::new();
        let mut lines = vec!["User: tool".to_string()];
        let mut ran = Vec::new();
        let mut last = String::new();

        for _ in 0..4 {
            let text = reply_for(&demo, &lines, 4242).expect("the call completes");
            let parsed = parse_reply(&text);
            assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
            let calls = calls_of(&parsed.items);
            last = text.clone();
            if calls.is_empty() {
                break;
            }
            lines.push(format!("Assistant: {text}"));
            for (index, call) in calls.iter().enumerate() {
                ran.push(call.tool.clone());
                let request = tool_call_t {
                    id: format!("c{index}"),
                    tool: call.tool.clone(),
                    args: call.args.clone(),
                };
                let outcome = runner
                    .run(&request, &cancel)
                    .expect("the simulated tools succeed");
                assert!(outcome.simulated);
                let first = outcome.output.lines().next().unwrap_or("");
                lines.push(format!("Tool {}: {first}", call.tool));
            }
        }

        assert_eq!(ran, vec!["list_dir", "read_file"]);
        assert!(last.contains("## Summary (simulated)"));
    }

    #[test]
    fn a_denied_edit_leads_to_the_denial_reply() {
        let demo = standard();
        let lines = vec![
            "User: approve".to_string(),
            "Assistant: I will edit.".to_string(),
            "Tool error replace_lines: denied by the user".to_string(),
        ];
        let text = reply_for(&demo, &lines, 4242).expect("completes");
        assert!(text.contains("was denied"), "{text}");
        assert!(!text.contains("@call"));
    }

    #[test]
    fn a_failed_read_leads_to_a_recovery_reply() {
        let demo = standard();
        let runner = demo_tool_runner_t::new(timing_t::instant());
        let cancel = cancel_token_t::new();
        let request = tool_call_t {
            id: "c0".to_string(),
            tool: "read_file".to_string(),
            args: vec![harness::tools::tool_arg_t {
                name: "path".to_string(),
                value: "docs/missing.txt".to_string(),
            }],
        };
        let err = runner
            .run(&request, &cancel)
            .expect_err("missing files fail");
        let lines = vec![
            "User: fail".to_string(),
            "Assistant: I will try.".to_string(),
            format!("Tool error read_file: {}", err.message),
        ];
        let text = reply_for(&demo, &lines, 4242).expect("completes");
        assert!(text.contains("failed"), "{text}");
        assert!(parse_reply(&text).issues.is_empty());
    }

    #[test]
    fn compact_long_reply_is_cut_and_still_parses_as_text() {
        let demo = demo_backend_t::new(ProfileKind::Compact, timing_t::instant());
        let lines = vec!["User: long".to_string()];
        let text = reply_for(&demo, &lines, 4242).expect("completes");
        assert_eq!(text.len(), 256);
        let parsed = parse_reply(&text);
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn hints_cover_every_trigger_once() {
        let hints = hints();
        let triggers: Vec<&str> = hints.iter().map(|hint| hint.trigger.as_str()).collect();
        assert_eq!(
            triggers,
            vec![
                "tool", "approve", "deny", "shell", "ask", "fail", "error", "flaky", "long",
                "markdown"
            ]
        );
        assert!(hints.iter().all(|hint| !hint.description.is_empty()));
    }

    #[test]
    fn the_notice_and_label_say_simulated() {
        assert!(Notice.starts_with("DEMO MODE"));
        assert!(Label.starts_with("DEMO"));
    }

    #[test]
    fn build_returns_a_demo_backend_and_runner_with_the_same_delays() {
        let (backend, runner) = build(ProfileKind::Compact, timing_t::instant());
        assert_eq!(backend.identity().id, "demo-compact");
        assert!(backend.identity().simulated());
        let cancel = cancel_token_t::new();
        let outcome = runner
            .run(
                &tool_call_t {
                    id: "c".to_string(),
                    tool: "list_dir".to_string(),
                    args: vec![harness::tools::tool_arg_t {
                        name: "path".to_string(),
                        value: ".".to_string(),
                    }],
                },
                &cancel,
            )
            .expect("list_dir succeeds");
        assert!(outcome.simulated);
        assert_eq!(byte_codec_t.encode("A"), vec![65]);
    }
}
