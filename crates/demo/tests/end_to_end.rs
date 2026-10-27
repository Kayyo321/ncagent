/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! End-to-end tests of the session controller, the demo backend and the demo tool runner
//! (PROJECT.md, "UI prototype (P1)"). Each test builds the runtime with `demo::build`, drives it
//! through `harness::api` and checks the events it returns. Every reply and tool result checked
//! here is simulated demo output (O1): no model runs, no command starts and no project file is
//! changed.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use demo::profile::ProfileKind;
use demo::store::file_store_t;
use demo::timing::timing_t;
use harness::api::{SessionError, SessionPhase, runtime_t, session_t};
use harness::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
use harness::backend::BackendError;
use harness::event::{
    CancelScope, ErrorScope, NoticeLevel, SessionEvent, TurnOutcome, stamped_event_t,
};
use harness::runtime::{
    DefaultSeed, MaxAttempts, PassBudget, controller_runtime_t, runtime_config_t, system_clock,
};
use harness::store::{StoreError, memory_store_t, session_store_t};
use harness::tools::ToolErrorKind;

/// Longest wait for a session to reach the state a test waits for.
const TimeoutSecs: u64 = 10;

/// Level lengths of the standard profile, in pass order. K is the length of this list.
const StandardLevelLens: [u32; 5] = [8, 32, 128, 512, 1024];

/// Level lengths of the compact profile, in pass order.
const CompactLevelLens: [u32; 4] = [16, 64, 128, 256];

/// Counts the scratch directories of this process, so two tests never share one.
static ScratchCounter: AtomicU64 = AtomicU64::new(0);

/// The settings of a run: the real demo notice and label, and the default budgets.
fn config(approval: ApprovalMode) -> runtime_config_t {
    runtime_config_t {
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        approval,
        session_seed: DefaultSeed,
        max_attempts: MaxAttempts,
        pass_budget: PassBudget,
        version: "test".to_string(),
        notice: Some(demo::Notice.to_string()),
        label: Some(demo::Label.to_string()),
        state_note: None,
        hints: demo::hints(),
        clock: system_clock,
    }
}

/// The real runtime over the demo backend and the demo tool runner, with the given store.
fn runtime_for(
    profile: ProfileKind,
    timing: timing_t,
    approval: ApprovalMode,
    store: Arc<dyn session_store_t>,
) -> controller_runtime_t {
    let (backend, tools) = demo::build(profile, timing);
    controller_runtime_t::new(backend, tools, store, config(approval))
        .expect("the demo backend has valid capabilities")
}

/// The runtime with no simulated delay and an in-memory store.
fn instant_runtime(profile: ProfileKind, approval: ApprovalMode) -> controller_runtime_t {
    runtime_for(
        profile,
        timing_t::instant(),
        approval,
        Arc::new(memory_store_t::new()),
    )
}

/// Pumps `session` until `done` holds, and returns every event pumped on the way. `done` sees the
/// session and the events pumped so far. The test fails when `TimeoutSecs` pass first.
fn pump_until(
    session: &mut dyn session_t,
    mut done: impl FnMut(&dyn session_t, &[stamped_event_t]) -> bool,
) -> Vec<stamped_event_t> {
    let started = Instant::now();
    let mut all = Vec::new();
    loop {
        all.extend(session.pump());
        if done(&*session, &all) {
            return all;
        }
        assert!(
            started.elapsed() < Duration::from_secs(TimeoutSecs),
            "timed out waiting, saw {:?}",
            names(&all)
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn pump_until_phase(session: &mut dyn session_t, wanted: SessionPhase) -> Vec<stamped_event_t> {
    pump_until(session, |current, _| current.phase() == wanted)
}

fn pump_until_idle(session: &mut dyn session_t) -> Vec<stamped_event_t> {
    pump_until_phase(session, SessionPhase::Idle)
}

/// Sends one message and pumps until the session is idle again.
fn run_turn(session: &mut dyn session_t, text: &str) -> Vec<stamped_event_t> {
    session.submit(text.to_string());
    pump_until_idle(session)
}

/// Collects what `pick` returns for each event, in order.
fn find_all<'a, T>(
    events: &'a [stamped_event_t],
    pick: impl Fn(&'a SessionEvent) -> Option<T>,
) -> Vec<T> {
    events
        .iter()
        .filter_map(|stamped| pick(&stamped.event))
        .collect()
}

fn names(events: &[stamped_event_t]) -> Vec<&'static str> {
    events
        .iter()
        .map(|stamped| event_name(&stamped.event))
        .collect()
}

fn event_name(event: &SessionEvent) -> &'static str {
    match event {
        SessionEvent::SessionStart { .. } => "session_start",
        SessionEvent::UserMessage { .. } => "user_message",
        SessionEvent::MessageQueued { .. } => "message_queued",
        SessionEvent::TurnStart { .. } => "turn_start",
        SessionEvent::CallStart { .. } => "call_start",
        SessionEvent::LevelDone { .. } => "level_done",
        SessionEvent::CallEnd { .. } => "call_end",
        SessionEvent::ToolCall { .. } => "tool_call",
        SessionEvent::ApprovalRequested { .. } => "approval_requested",
        SessionEvent::Approval { .. } => "approval",
        SessionEvent::ToolResult { .. } => "tool_result",
        SessionEvent::ToolError { .. } => "tool_error",
        SessionEvent::QuestionAsked { .. } => "question_asked",
        SessionEvent::AskAnswer { .. } => "ask_answer",
        SessionEvent::ApprovalModeChanged { .. } => "approval_mode_changed",
        SessionEvent::Cancel { .. } => "cancel",
        SessionEvent::Error { .. } => "error",
        SessionEvent::Notice { .. } => "notice",
        SessionEvent::TurnEnd { .. } => "turn_end",
    }
}

/// The text of the last `call_end`, which is the reply the user reads.
fn final_text(events: &[stamped_event_t]) -> String {
    find_all(events, |event| match event {
        SessionEvent::CallEnd { text, .. } => Some(text.clone()),
        _ => None,
    })
    .last()
    .cloned()
    .expect("the turn produced a reply")
}

/// The outcome, call count and pass count of the last `turn_end`.
fn last_turn_end(events: &[stamped_event_t]) -> (TurnOutcome, u32, u32) {
    find_all(events, |event| match event {
        SessionEvent::TurnEnd {
            outcome,
            calls,
            passes,
            ..
        } => Some((*outcome, *calls, *passes)),
        _ => None,
    })
    .last()
    .copied()
    .expect("the turn ended")
}

/// One `level_done` event, as the UI reads it.
struct level_row_t {
    level: u32,
    level_count: u32,
    level_len: u32,
    draft: Option<String>,
}

fn level_rows(events: &[stamped_event_t]) -> Vec<level_row_t> {
    find_all(events, |event| match event {
        SessionEvent::LevelDone {
            level,
            level_count,
            level_len,
            draft,
            ..
        } => Some(level_row_t {
            level: *level,
            level_count: *level_count,
            level_len: *level_len,
            draft: draft.clone(),
        }),
        _ => None,
    })
}

/// The id of the first approval request in `events`.
fn first_request_id(events: &[stamped_event_t]) -> String {
    find_all(events, |event| match event {
        SessionEvent::ApprovalRequested { tool_call_id, .. } => Some(tool_call_id.clone()),
        _ => None,
    })
    .into_iter()
    .next()
    .expect("an approval was requested")
}

/// The id of the first question in `events`.
fn first_question_id(events: &[stamped_event_t]) -> String {
    find_all(events, |event| match event {
        SessionEvent::QuestionAsked { question_id, .. } => Some(question_id.clone()),
        _ => None,
    })
    .into_iter()
    .next()
    .expect("a question was asked")
}

/// A directory under the system temp directory, removed when the value is dropped.
struct scratch_dir_t {
    path: PathBuf,
}

impl scratch_dir_t {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        let serial = ScratchCounter.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "ncagent-demo-e2e-{}-{nanos}-{serial}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("a fresh scratch directory");
        scratch_dir_t { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for scratch_dir_t {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn default_prompt_runs_the_standard_profile() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "hello");

    assert_eq!(
        names(&events),
        [
            "session_start",
            "user_message",
            "turn_start",
            "call_start",
            "level_done",
            "level_done",
            "level_done",
            "level_done",
            "level_done",
            "call_end",
            "turn_end",
        ]
    );

    let SessionEvent::SessionStart {
        session_id,
        simulated,
        label,
        level_count,
        level_lens,
        ..
    } = &events[0].event
    else {
        panic!("the first event is session_start");
    };
    assert_eq!(session_id, session.id());
    assert!(*simulated, "the demo session is marked simulated");
    assert_eq!(label.as_deref(), Some(demo::Label));
    assert_eq!(*level_count, 5);
    assert_eq!(level_lens.as_slice(), StandardLevelLens.as_slice());

    let rows = level_rows(&events);
    let levels: Vec<u32> = rows.iter().map(|row| row.level).collect();
    assert_eq!(levels, [0, 1, 2, 3, 4]);
    let lens: Vec<u32> = rows.iter().map(|row| row.level_len).collect();
    assert_eq!(lens.as_slice(), StandardLevelLens.as_slice());
    assert!(rows.iter().all(|row| row.level_count == 5));
    assert!(
        rows.iter().all(|row| row.draft.is_some()),
        "the standard profile sends drafts"
    );

    let ends = find_all(&events, |event| match event {
        SessionEvent::CallEnd {
            valid,
            truncated,
            passes,
            ..
        } => Some((*valid, *truncated, *passes)),
        _ => None,
    });
    assert_eq!(ends, [(true, false, 5)]);
    assert_eq!(
        last_turn_end(&events),
        (TurnOutcome::Completed, 1, 5),
        "one call of five passes"
    );
    assert!(final_text(&events).starts_with("## Demo reply"));
}

#[test]
fn compact_profile_has_four_levels_and_no_drafts() {
    let runtime = instant_runtime(ProfileKind::Compact, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "hello");

    let SessionEvent::SessionStart {
        level_count,
        level_lens,
        ..
    } = &events[0].event
    else {
        panic!("the first event is session_start");
    };
    assert_eq!(*level_count, 4);
    assert_eq!(level_lens.as_slice(), CompactLevelLens.as_slice());

    let rows = level_rows(&events);
    let lens: Vec<u32> = rows.iter().map(|row| row.level_len).collect();
    assert_eq!(lens.as_slice(), CompactLevelLens.as_slice());
    assert!(rows.iter().all(|row| row.level_count == 4));
    assert!(
        rows.iter().all(|row| row.draft.is_none()),
        "the compact profile sends no drafts"
    );
    assert_eq!(last_turn_end(&events), (TurnOutcome::Completed, 1, 4));
}

#[test]
fn tool_turn_runs_two_read_tools_without_asking() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "tool");

    let tools = find_all(&events, |event| match event {
        SessionEvent::ToolCall { tool, .. } => Some(tool.clone()),
        _ => None,
    });
    assert_eq!(tools, ["list_dir", "read_file"]);

    let approvals = find_all(&events, |event| match event {
        SessionEvent::Approval {
            decision, source, ..
        } => Some((*decision, *source)),
        _ => None,
    });
    assert_eq!(
        approvals,
        [
            (ApprovalDecision::Once, ApprovalSource::Mode),
            (ApprovalDecision::Once, ApprovalSource::Mode),
        ],
        "read tools are allowed by the mode"
    );
    assert!(
        !names(&events).contains(&"approval_requested"),
        "no read tool asks"
    );

    let results = find_all(&events, |event| match event {
        SessionEvent::ToolResult { ok, simulated, .. } => Some((*ok, *simulated)),
        _ => None,
    });
    assert_eq!(results, [(true, true), (true, true)]);

    let starts = find_all(&events, |event| match event {
        SessionEvent::CallStart { call, attempt, .. } => Some((*call, *attempt)),
        _ => None,
    });
    assert_eq!(starts, [(0, 0), (1, 0), (2, 0)]);

    assert_eq!(
        last_turn_end(&events),
        (TurnOutcome::Completed, 3, 15),
        "three calls of five passes"
    );
    assert!(final_text(&events).contains("## Summary (simulated)"));
}

#[test]
fn approved_edit_runs_after_the_user_says_once() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    session.submit("approve".to_string());
    let mut events = pump_until_phase(session.as_mut(), SessionPhase::AwaitingApproval);

    let requests = find_all(&events, |event| match event {
        SessionEvent::ApprovalRequested { tool, summary, .. } => {
            Some((tool.clone(), summary.clone()))
        }
        _ => None,
    });
    assert_eq!(requests.len(), 1, "one edit asks for approval");
    assert_eq!(requests[0].0, "replace_lines");
    assert!(
        requests[0].1.contains("src/lib.rs"),
        "the summary names the path: {}",
        requests[0].1
    );

    let call_id = first_request_id(&events);
    session.resolve_approval(&call_id, ApprovalDecision::Once);
    events.extend(pump_until_idle(session.as_mut()));

    let decisions = find_all(&events, |event| match event {
        SessionEvent::Approval {
            tool_call_id,
            decision,
            source,
        } => Some((tool_call_id.clone(), *decision, *source)),
        _ => None,
    });
    assert_eq!(
        decisions,
        [(
            call_id.clone(),
            ApprovalDecision::Once,
            ApprovalSource::User
        )]
    );

    let results = find_all(&events, |event| match event {
        SessionEvent::ToolResult { ok, simulated, .. } => Some((*ok, *simulated)),
        _ => None,
    });
    assert_eq!(results, [(true, true)]);
    assert!(final_text(&events).contains("applied"));
    assert_eq!(last_turn_end(&events).0, TurnOutcome::Completed);
}

#[test]
fn denied_edit_leads_to_a_denial_reply() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    session.submit("deny".to_string());
    let mut events = pump_until_phase(session.as_mut(), SessionPhase::AwaitingApproval);

    let call_id = first_request_id(&events);
    session.resolve_approval(&call_id, ApprovalDecision::Deny);
    events.extend(pump_until_idle(session.as_mut()));

    let errors = find_all(&events, |event| match event {
        SessionEvent::ToolError {
            tool_call_id,
            kind,
            message,
            ..
        } => Some((tool_call_id.clone(), *kind, message.clone())),
        _ => None,
    });
    assert_eq!(
        errors,
        [(
            call_id.clone(),
            ToolErrorKind::Denied,
            "denied by user".to_string()
        )]
    );
    assert!(
        !names(&events).contains(&"tool_result"),
        "a denied edit runs nothing"
    );
    assert!(final_text(&events).contains("denied"));
    assert_eq!(last_turn_end(&events).0, TurnOutcome::Completed);
}

#[test]
fn edit_mode_allows_a_write_inside_the_project_and_still_asks_for_shell() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Edit);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "approve");

    assert!(
        !names(&events).contains(&"approval_requested"),
        "an edit inside the working directory needs no prompt in edit mode"
    );
    let sources = find_all(&events, |event| match event {
        SessionEvent::Approval { source, .. } => Some(*source),
        _ => None,
    });
    assert_eq!(sources, [ApprovalSource::Mode]);
    assert!(final_text(&events).contains("applied"));

    session.submit("shell".to_string());
    let shell = pump_until_phase(session.as_mut(), SessionPhase::AwaitingApproval);
    let requests = find_all(&shell, |event| match event {
        SessionEvent::ApprovalRequested { tool, summary, .. } => {
            Some((tool.clone(), summary.clone()))
        }
        _ => None,
    });
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, "run_shell");
    assert!(requests[0].1.contains("echo"));

    session.cancel();
    let rest = pump_until_idle(session.as_mut());
    let scopes = find_all(&rest, |event| match event {
        SessionEvent::Cancel { scope, .. } => Some(*scope),
        _ => None,
    });
    assert_eq!(scopes, [CancelScope::Approval]);
    assert_eq!(last_turn_end(&rest).0, TurnOutcome::Cancelled);
}

#[test]
fn question_is_answered_and_the_reply_repeats_the_answer() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    session.submit("ask".to_string());
    let mut events = pump_until_phase(session.as_mut(), SessionPhase::AwaitingAnswer);

    let questions = find_all(&events, |event| match event {
        SessionEvent::QuestionAsked { options, .. } => Some(options.clone()),
        _ => None,
    });
    assert_eq!(questions.len(), 1);
    let options: Vec<&str> = questions[0].iter().map(String::as_str).collect();
    assert_eq!(options, ["coarse", "fine", "both"]);

    let question_id = first_question_id(&events);
    session.answer_question(&question_id, "fine".to_string(), Some(1));
    events.extend(pump_until_idle(session.as_mut()));

    let answers = find_all(&events, |event| match event {
        SessionEvent::AskAnswer {
            answer,
            option_index,
            ..
        } => Some((answer.clone(), *option_index)),
        _ => None,
    });
    assert_eq!(answers, [("fine".to_string(), Some(1))]);
    assert!(final_text(&events).contains("You chose \"fine\""));
    assert_eq!(last_turn_end(&events), (TurnOutcome::Completed, 2, 10));
}

#[test]
fn failed_read_is_a_tool_error_and_the_turn_still_completes() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "fail");

    let errors = find_all(&events, |event| match event {
        SessionEvent::ToolError { kind, message, .. } => Some((*kind, message.clone())),
        _ => None,
    });
    assert_eq!(errors.len(), 1, "the missing file is one tool error");
    assert_eq!(errors[0].0, ToolErrorKind::Failed);
    assert!(
        errors[0].1.contains("simulated"),
        "the message says it is simulated: {}",
        errors[0].1
    );
    assert!(final_text(&events).contains("failed"));
    assert_eq!(last_turn_end(&events), (TurnOutcome::Completed, 2, 10));
}

#[test]
fn backend_failure_ends_the_turn_and_the_next_message_completes() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let failed = run_turn(session.as_mut(), "error");

    let expected_name = BackendError::Failed {
        message: String::new(),
    }
    .name();
    assert_eq!(expected_name, "backend_failed");

    let errors = find_all(&failed, |event| match event {
        SessionEvent::Error {
            scope,
            name,
            message,
            ..
        } => Some((*scope, name.clone(), message.clone())),
        _ => None,
    });
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, ErrorScope::Backend);
    assert_eq!(errors[0].1, expected_name);
    assert!(
        errors[0].2.contains("simulated backend failure"),
        "{}",
        errors[0].2
    );
    assert_eq!(last_turn_end(&failed).0, TurnOutcome::Failed);
    assert_eq!(session.phase(), SessionPhase::Idle);

    let recovered = run_turn(session.as_mut(), "hello");
    assert_eq!(
        last_turn_end(&recovered),
        (TurnOutcome::Completed, 1, 5),
        "the session recovers after a failed turn"
    );
    let turns = find_all(&recovered, |event| match event {
        SessionEvent::UserMessage { turn, .. } => Some(*turn),
        _ => None,
    });
    assert_eq!(turns, [2]);
}

#[test]
fn flaky_first_sample_is_retried_with_the_next_seed() {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "flaky");

    let starts = find_all(&events, |event| match event {
        SessionEvent::CallStart {
            call,
            attempt,
            seed,
            ..
        } => Some((*call, *attempt, *seed)),
        _ => None,
    });
    assert_eq!(starts.len(), 2, "one call, two attempts");
    assert_eq!(starts[0].0, 0);
    assert_eq!(starts[1].0, 0);
    assert_eq!(starts[0].1, 0);
    assert_eq!(starts[1].1, 1);
    let first_seed = starts[0].2;
    assert_eq!(
        first_seed % 2,
        0,
        "the first sample of a call has an even seed (D42)"
    );
    assert_eq!(starts[1].2, first_seed + 1, "the retry uses the next seed");

    let ends = find_all(&events, |event| match event {
        SessionEvent::CallEnd {
            attempt,
            valid,
            issues,
            ..
        } => Some((*attempt, *valid, issues.is_empty())),
        _ => None,
    });
    assert_eq!(ends, [(0, false, false), (1, true, true)]);
    assert_eq!(last_turn_end(&events), (TurnOutcome::Completed, 1, 10));
}

#[test]
fn cancel_between_level_passes_ends_the_turn_cancelled() {
    let timing = timing_t {
        level_ms: 50,
        tool_ms: 0,
    };
    let runtime = runtime_for(
        ProfileKind::Standard,
        timing,
        ApprovalMode::Ask,
        Arc::new(memory_store_t::new()),
    );
    let mut session = runtime.new_session().expect("a session starts");
    session.submit("hello".to_string());
    let mut events = pump_until(session.as_mut(), |_, all| {
        all.iter()
            .any(|stamped| matches!(stamped.event, SessionEvent::LevelDone { .. }))
    });

    session.cancel();
    events.extend(pump_until_idle(session.as_mut()));

    let cancels = find_all(&events, |event| match event {
        SessionEvent::Cancel {
            call,
            levels_done,
            scope,
            ..
        } => Some((*call, *levels_done, *scope)),
        _ => None,
    });
    assert_eq!(cancels.len(), 1, "one cancel event");
    let (call, levels_done, scope) = cancels[0];
    assert_eq!(scope, CancelScope::Call);
    assert_eq!(call, Some(0));
    let done = levels_done.expect("the cancel reports the levels done");
    assert!(
        (1..5).contains(&done),
        "the call stopped after some passes and before the fifth, saw {done}"
    );
    assert_eq!(last_turn_end(&events).0, TurnOutcome::Cancelled);
    assert!(
        !names(&events).contains(&"call_end"),
        "a cancelled call has no final block"
    );
}

#[test]
fn long_reply_is_truncated_under_the_compact_profile_with_a_notice() {
    let runtime = instant_runtime(ProfileKind::Compact, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let events = run_turn(session.as_mut(), "long");

    let ends = find_all(&events, |event| match event {
        SessionEvent::CallEnd {
            truncated, valid, ..
        } => Some((*truncated, *valid)),
        _ => None,
    });
    assert_eq!(ends, [(true, true)], "the reply is cut and still parses");

    let notices = find_all(&events, |event| match event {
        SessionEvent::Notice { level, text } => Some((*level, text.clone())),
        _ => None,
    });
    assert!(
        notices
            .iter()
            .any(|(level, text)| *level == NoticeLevel::Warning && text.contains("D11")),
        "a warning names D11, saw {notices:?}"
    );
    assert_eq!(last_turn_end(&events), (TurnOutcome::Completed, 1, 4));
}

/// Runs the prompts in one new session of a fresh runtime. Returns the session id and the events.
fn run_prompts(prompts: &[&str]) -> (String, Vec<stamped_event_t>) {
    let runtime = instant_runtime(ProfileKind::Standard, ApprovalMode::Ask);
    let mut session = runtime.new_session().expect("a session starts");
    let id = session.id().to_string();
    let mut events = Vec::new();
    for prompt in prompts {
        events.extend(run_turn(session.as_mut(), prompt));
    }
    (id, events)
}

/// The events as JSON, with the time of each set to zero and the session id of `session_start`
/// removed. What is left must match between two runs with the same seed and prompts.
fn normalized(events: &[stamped_event_t]) -> Vec<serde_json::Value> {
    events
        .iter()
        .map(|stamped| {
            let mut value = serde_json::to_value(stamped).expect("an event serializes");
            if let Some(object) = value.as_object_mut() {
                object.insert("at_ms".to_string(), serde_json::json!(0));
            }
            if let Some(inner) = value
                .get_mut("event")
                .and_then(serde_json::Value::as_object_mut)
            {
                inner.remove("session_id");
            }
            value
        })
        .collect()
}

#[test]
fn same_seed_and_prompts_give_the_same_events() {
    let prompts = ["hello", "tool"];
    let (first_id, first) = run_prompts(&prompts);
    let (second_id, second) = run_prompts(&prompts);

    assert_ne!(first_id, second_id, "two sessions have two ids");
    assert!(!first.is_empty());
    assert_eq!(normalized(&first), normalized(&second));
}

#[test]
fn stored_tool_session_lists_reopens_and_continues() {
    let dir = scratch_dir_t::new();
    let store: Arc<dyn session_store_t> = Arc::new(file_store_t::new(dir.path().to_path_buf()));
    let runtime = runtime_for(
        ProfileKind::Standard,
        timing_t::instant(),
        ApprovalMode::Ask,
        Arc::clone(&store),
    );
    let mut session = runtime.new_session().expect("a session starts");
    let id = session.id().to_string();
    let written = run_turn(session.as_mut(), "tool");
    drop(session);

    let listed = runtime.list_sessions().expect("the store lists sessions");
    assert_eq!(listed.len(), 1, "one stored session");
    assert_eq!(listed[0].id, id);
    assert!(
        listed[0].simulated,
        "the stored session is marked simulated"
    );
    assert_eq!(listed[0].label.as_deref(), Some(demo::Label));
    assert_eq!(listed[0].turns, 1);

    let opened = runtime.open_session(&id).expect("the session reopens");
    assert_eq!(
        opened.events, written,
        "the replayed events match the live ones"
    );
    assert_eq!(opened.skipped_lines, 0);

    let mut reopened = opened.session;
    let last_seq = written.last().expect("events were written").seq;
    let next = run_turn(reopened.as_mut(), "hello");
    let first_seq = next.first().expect("the new turn wrote events").seq;
    assert_eq!(first_seq, last_seq + 1, "sequence numbers continue");

    let turns = find_all(&next, |event| match event {
        SessionEvent::TurnStart { turn } => Some(*turn),
        _ => None,
    });
    assert_eq!(turns, [2], "the turn number continues");
    let calls = find_all(&next, |event| match event {
        SessionEvent::CallStart { call, .. } => Some(*call),
        _ => None,
    });
    assert_eq!(
        calls,
        [3],
        "the call number continues after the three calls"
    );
}

#[test]
fn hostile_session_id_is_refused_without_a_panic() {
    let dir = scratch_dir_t::new();
    let store: Arc<dyn session_store_t> = Arc::new(file_store_t::new(dir.path().to_path_buf()));
    let runtime = runtime_for(
        ProfileKind::Standard,
        timing_t::instant(),
        ApprovalMode::Ask,
        store,
    );

    let result = runtime.open_session("../etc/passwd");
    assert!(
        matches!(
            result,
            Err(SessionError::Store(StoreError::InvalidId { .. }))
        ),
        "an id with a path separator is refused as an invalid id"
    );
}
