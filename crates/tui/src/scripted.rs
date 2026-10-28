/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! UI PREVIEW FIXTURE (scripted events). This is not a backend and not a simulator of a model.
//! It hands the UI fixed `SessionEvent` sequences so the interface can be seen and tested
//! without the harness controller. It is compiled only for tests and with the `preview`
//! feature (the gallery example); the shipped binary never contains it.
//!
//! Two modes:
//! - manual: a test pushes events through a [`script_handle_t`] and reads back every call the
//!   UI made (`submit`, `cancel`, `resolve_approval`, ...);
//! - preview: each message plays one canned scenario on a timer (levels, the dissolve, a tool
//!   call, an approval, a question, a retry), waiting for the user where a real turn would.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use harness::api::{
    SessionError, SessionPhase, hint_t, opened_session_t, runtime_info_t, runtime_t, session_t,
};
use harness::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
use harness::backend::{BackendKind, capabilities_t, model_identity_t, sampler_t};
use harness::event::{CancelScope, ReplyItem, SessionEvent, TurnOutcome, stamped_event_t};
use harness::organize::{OrganizeOp, organization_t};
use harness::settings::{
    custom_theme_t, loaded_themes_t, memory_settings_store_t, settings_store_t, settings_t,
};
use harness::store::{session_summary_t, summarize_events};
use harness::tools::{ApprovalClass, ToolErrorKind, parsed_call_t, tool_arg_t};

/// The label shown wherever this fixture runs.
pub const PreviewLabel: &str = "UI PREVIEW (scripted events)";

/// A call the UI made on a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptCall {
    Submit(String),
    Cancel,
    ResolveApproval(String, ApprovalDecision),
    AnswerQuestion(String, String, Option<u32>),
    ClearQueue,
    SetApprovalMode(ApprovalMode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Manual,
    Preview,
}

/// What a preview scenario waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Wait {
    None,
    Approval { id: String, then: usize },
    Question { id: String, then: usize },
}

struct step_t {
    delay: Duration,
    event: SessionEvent,
}

/// State shared by a session and its test handle.
pub struct shared_t {
    incoming: VecDeque<SessionEvent>,
    pub calls: Vec<ScriptCall>,
    pub phase: SessionPhase,
    pub queue: Vec<String>,
    pub mode: ApprovalMode,
    seq: u64,
    // Preview mode.
    steps: VecDeque<step_t>,
    due: Option<Instant>,
    wait: Wait,
    turn: u32,
    /// Call index of the reply that follows an approval or an answer.
    next_call: usize,
    levels_done: u32,
}

/// A test's view of one scripted session.
#[derive(Clone)]
pub struct script_handle_t {
    pub id: String,
    shared: Arc<Mutex<shared_t>>,
}

impl script_handle_t {
    /// Queue events for the next `pump`.
    pub fn push(&self, events: impl IntoIterator<Item = SessionEvent>) {
        if let Ok(mut s) = self.shared.lock() {
            s.incoming.extend(events);
        }
    }

    /// Every call the UI made on this session, oldest first.
    pub fn calls(&self) -> Vec<ScriptCall> {
        self.shared
            .lock()
            .map(|s| s.calls.clone())
            .unwrap_or_default()
    }

    pub fn set_phase(&self, phase: SessionPhase) {
        if let Ok(mut s) = self.shared.lock() {
            s.phase = phase;
        }
    }
}

fn phase_after(event: &SessionEvent, current: SessionPhase) -> SessionPhase {
    match event {
        SessionEvent::TurnStart { .. }
        | SessionEvent::CallStart { .. }
        | SessionEvent::LevelDone { .. }
        | SessionEvent::ToolResult { .. }
        | SessionEvent::ToolError { .. }
        | SessionEvent::AskAnswer { .. } => SessionPhase::Generating,
        SessionEvent::ApprovalRequested { .. } => SessionPhase::AwaitingApproval,
        SessionEvent::Approval { decision, .. } => match decision {
            ApprovalDecision::Deny => SessionPhase::Generating,
            _ => SessionPhase::RunningTool,
        },
        SessionEvent::ToolCall { .. } => SessionPhase::RunningTool,
        SessionEvent::QuestionAsked { .. } => SessionPhase::AwaitingAnswer,
        SessionEvent::TurnEnd { .. } => SessionPhase::Idle,
        _ => current,
    }
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct scripted_session_t {
    id: String,
    mode: Mode,
    shared: Arc<Mutex<shared_t>>,
}

impl scripted_session_t {
    fn stamp(s: &mut shared_t, mode: Mode, event: SessionEvent) -> stamped_event_t {
        let seq = s.seq;
        s.seq = s.seq.saturating_add(1);
        let at_ms = match mode {
            Mode::Manual => 1_000 + seq * 100,
            Mode::Preview => wall_ms(),
        };
        s.phase = phase_after(&event, s.phase);
        stamped_event_t { seq, at_ms, event }
    }
}

impl session_t for scripted_session_t {
    fn id(&self) -> &str {
        &self.id
    }

    fn submit(&mut self, text: String) {
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        s.calls.push(ScriptCall::Submit(text.clone()));
        if self.mode == Mode::Preview {
            if s.phase.busy() {
                s.queue.push(text.clone());
                s.incoming.push_back(SessionEvent::MessageQueued { text });
            } else {
                start_turn(&mut s, text, false);
            }
        } else if s.phase.busy() {
            s.queue.push(text);
        }
    }

    fn cancel(&mut self) {
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        s.calls.push(ScriptCall::Cancel);
        if self.mode == Mode::Preview && s.phase.busy() {
            let turn = s.turn;
            let scope = match s.wait {
                Wait::Approval { .. } => CancelScope::Approval,
                Wait::Question { .. } => CancelScope::Question,
                Wait::None => CancelScope::Call,
            };
            s.steps.clear();
            s.wait = Wait::None;
            let levels_done = s.levels_done;
            s.incoming.push_back(SessionEvent::Cancel {
                turn,
                call: None,
                levels_done: (scope == CancelScope::Call).then_some(levels_done),
                scope,
            });
            s.incoming.push_back(SessionEvent::TurnEnd {
                turn,
                outcome: TurnOutcome::Cancelled,
                calls: 1,
                passes: levels_done,
            });
            s.phase = SessionPhase::Cancelling;
        }
    }

    fn resolve_approval(&mut self, tool_call_id: &str, decision: ApprovalDecision) {
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        s.calls.push(ScriptCall::ResolveApproval(
            tool_call_id.to_string(),
            decision,
        ));
        if let Wait::Approval { id, then } = s.wait.clone()
            && id == tool_call_id
        {
            s.wait = Wait::None;
            let turn = s.turn;
            let approved = decision != ApprovalDecision::Deny;
            s.steps.push_back(step_t {
                delay: Duration::ZERO,
                event: SessionEvent::Approval {
                    tool_call_id: id.clone(),
                    decision,
                    source: ApprovalSource::User,
                },
            });
            if approved {
                s.steps.push_back(step_t {
                    delay: Duration::from_millis(600),
                    event: SessionEvent::ToolResult {
                        turn,
                        tool_call_id: id.clone(),
                        tool: "replace_lines".into(),
                        ok: true,
                        output: "replaced lines 12-14 of src/greeting.rs\n(preview fixture: nothing was written)".into(),
                        truncated: false,
                        simulated: true,
                    },
                });
            } else {
                s.steps.push_back(step_t {
                    delay: Duration::from_millis(200),
                    event: SessionEvent::ToolError {
                        turn,
                        tool_call_id: id,
                        tool: "replace_lines".into(),
                        kind: ToolErrorKind::Denied,
                        message: "the user denied the change".into(),
                    },
                });
            }
            let text = if approved {
                "The change is in place. *(Scripted preview text.)*"
            } else {
                "Understood, I left the file as it was. *(Scripted preview text.)*"
            };
            reply_steps(&mut s, turn, then as u32, text, true, &[]);
            end_turn_steps(&mut s, turn, then as u32 + 1);
        }
    }

    fn answer_question(&mut self, question_id: &str, answer: String, option_index: Option<u32>) {
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        s.calls.push(ScriptCall::AnswerQuestion(
            question_id.to_string(),
            answer.clone(),
            option_index,
        ));
        if let Wait::Question { id, then } = s.wait.clone()
            && id == question_id
        {
            s.wait = Wait::None;
            let turn = s.turn;
            s.steps.push_back(step_t {
                delay: Duration::ZERO,
                event: SessionEvent::AskAnswer {
                    turn,
                    question_id: id,
                    answer: answer.clone(),
                    option_index,
                },
            });
            let text = format!(
                "You chose **{}**. A real turn would continue from here. *(Scripted preview text.)*",
                answer.replace(['*', '`', '_'], "")
            );
            reply_steps(&mut s, turn, then as u32, &text, true, &[]);
            end_turn_steps(&mut s, turn, then as u32 + 1);
        }
    }

    fn queued(&self) -> Vec<String> {
        self.shared
            .lock()
            .map(|s| s.queue.clone())
            .unwrap_or_default()
    }

    fn clear_queue(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.calls.push(ScriptCall::ClearQueue);
            s.queue.clear();
        }
    }

    fn set_approval_mode(&mut self, mode: ApprovalMode) -> Result<(), SessionError> {
        let Ok(mut s) = self.shared.lock() else {
            return Err(SessionError::Internal {
                message: "fixture lock".into(),
            });
        };
        s.calls.push(ScriptCall::SetApprovalMode(mode));
        if mode == ApprovalMode::Full {
            return Err(SessionError::FullRequiresFlag);
        }
        s.mode = mode;
        s.incoming
            .push_back(SessionEvent::ApprovalModeChanged { mode });
        Ok(())
    }

    fn approval_mode(&self) -> ApprovalMode {
        self.shared
            .lock()
            .map(|s| s.mode)
            .unwrap_or(ApprovalMode::Ask)
    }

    fn phase(&self) -> SessionPhase {
        self.shared
            .lock()
            .map(|s| s.phase)
            .unwrap_or(SessionPhase::Idle)
    }

    fn pump(&mut self) -> Vec<stamped_event_t> {
        let Ok(mut s) = self.shared.lock() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while let Some(e) = s.incoming.pop_front() {
            out.push(Self::stamp(&mut s, self.mode, e));
        }
        if self.mode == Mode::Preview {
            let now = Instant::now();
            loop {
                let Some(delay) = s.steps.front().map(|st| st.delay) else {
                    s.due = None;
                    break;
                };
                let due = *s.due.get_or_insert(now + delay);
                if now < due {
                    break;
                }
                s.due = None;
                let Some(step) = s.steps.pop_front() else {
                    break;
                };
                match &step.event {
                    SessionEvent::LevelDone { .. } => {
                        s.levels_done = s.levels_done.saturating_add(1)
                    }
                    SessionEvent::TurnStart { .. } => s.levels_done = 0,
                    SessionEvent::ApprovalRequested { tool_call_id, .. } => {
                        let then = s.next_call;
                        s.wait = Wait::Approval {
                            id: tool_call_id.clone(),
                            then,
                        };
                    }
                    SessionEvent::QuestionAsked { question_id, .. } => {
                        let then = s.next_call;
                        s.wait = Wait::Question {
                            id: question_id.clone(),
                            then,
                        };
                    }
                    _ => {}
                }
                let ended = matches!(step.event, SessionEvent::TurnEnd { .. });
                out.push(Self::stamp(&mut s, self.mode, step.event));
                if ended && !s.queue.is_empty() {
                    let next = s.queue.remove(0);
                    start_turn(&mut s, next, true);
                }
            }
        }
        out
    }
}

/// Drafts that coarsen a text: at level `level` of `count`, neighbouring words are merged into
/// one token, so a coarse level gives fewer, wider blocks. The last level is the text itself.
pub fn coarse_draft(text: &str, level: u32, count: u32) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let group = 1usize << count.saturating_sub(1).saturating_sub(level).min(6);
    words
        .chunks(group.max(1))
        .map(|c| c.concat())
        .collect::<Vec<_>>()
        .join(" ")
}

const LevelLens: [u32; 5] = [8, 32, 128, 512, 1024];

fn level_steps(s: &mut shared_t, turn: u32, call: u32, attempt: u32, text: &str, drafts: bool) {
    let count = LevelLens.len() as u32;
    for (i, len) in LevelLens.iter().enumerate() {
        let level = i as u32;
        s.steps.push_back(step_t {
            delay: Duration::from_millis(560 + 40 * u64::from(level)),
            event: SessionEvent::LevelDone {
                turn,
                call,
                attempt,
                level,
                level_count: count,
                level_len: *len,
                elapsed_ms: 560 + 40 * u64::from(level),
                draft: drafts.then(|| coarse_draft(text, level, count)),
            },
        });
    }
}

fn call_start(turn: u32, call: u32, attempt: u32) -> SessionEvent {
    SessionEvent::CallStart {
        turn,
        call,
        attempt,
        seed: 8484 + 1024 * u64::from(call) + u64::from(attempt),
        prompt_hash: "preview".into(),
        packed_tokens: 900 + 310 * turn + 120 * call,
        window: 4096,
        dropped_turns: 0,
        sampler: sampler_t::greedy(),
    }
}

/// A complete call whose reply is `text`, followed by `items` (tool calls or a question).
fn reply_steps(
    s: &mut shared_t,
    turn: u32,
    call: u32,
    text: &str,
    drafts: bool,
    items: &[ReplyItem],
) {
    s.steps.push_back(step_t {
        delay: Duration::from_millis(250),
        event: call_start(turn, call, 0),
    });
    level_steps(s, turn, call, 0, text, drafts);
    let mut all = vec![ReplyItem::Text { text: text.into() }];
    all.extend(items.iter().cloned());
    s.steps.push_back(step_t {
        delay: Duration::from_millis(80),
        event: SessionEvent::CallEnd {
            turn,
            call,
            attempt: 0,
            text: text.into(),
            truncated: false,
            valid: true,
            issues: Vec::new(),
            items: all,
            passes: LevelLens.len() as u32,
        },
    });
}

fn end_turn_steps(s: &mut shared_t, turn: u32, calls: u32) {
    s.steps.push_back(step_t {
        delay: Duration::from_millis(300),
        event: SessionEvent::TurnEnd {
            turn,
            outcome: TurnOutcome::Completed,
            calls,
            passes: calls * LevelLens.len() as u32,
        },
    });
}

const IntroReply: &str = "## Hello from the UI preview\n\nThis reply arrived **level by level**: the blocks above were shaped by each level's draft, and the words dissolved in once the last level was done.\n\n- tool calls are pink-purple, with a status and an expandable result\n- approvals are amber and wait for you\n- questions offer numbered options\n\n| shown | from |\n|---|---|\n| level strip | each `level_done` |\n| dissolve | the final `call_end` |\n\n```rust\nfn main() {\n    println!(\"scripted preview text\");\n}\n```\n\nType *tool*, *approve*, *ask* or *retry* to see the other scenarios. This is scripted preview text, not model output.";

fn start_turn(s: &mut shared_t, text: String, from_queue: bool) {
    s.turn = s.turn.saturating_add(1);
    let turn = s.turn;
    s.steps.push_back(step_t {
        delay: Duration::ZERO,
        event: SessionEvent::TurnStart { turn },
    });
    s.steps.push_back(step_t {
        delay: Duration::ZERO,
        event: SessionEvent::UserMessage {
            turn,
            text: text.clone(),
            from_queue,
        },
    });
    let lower = text.to_lowercase();
    let scenario = if lower.contains("tool") {
        1
    } else if lower.contains("approve") || lower.contains("deny") {
        2
    } else if lower.contains("ask") {
        3
    } else if lower.contains("retry") {
        4
    } else if turn == 1 {
        0
    } else {
        (turn as usize - 1) % 5
    };
    match scenario {
        1 => {
            let call = ReplyItem::Call {
                call: parsed_call_t {
                    tool: "read_file".into(),
                    args: vec![tool_arg_t {
                        name: "path".into(),
                        value: "src/main.rs".into(),
                    }],
                },
            };
            reply_steps(
                s,
                turn,
                0,
                "Let me look at the entry point first.",
                true,
                &[call],
            );
            s.steps.push_back(step_t {
                delay: Duration::from_millis(120),
                event: SessionEvent::ToolCall {
                    turn,
                    tool_call_id: format!("t{turn}"),
                    tool: "read_file".into(),
                    class: ApprovalClass::Read,
                    args: vec![tool_arg_t {
                        name: "path".into(),
                        value: "src/main.rs".into(),
                    }],
                },
            });
            s.steps.push_back(step_t {
                delay: Duration::ZERO,
                event: SessionEvent::Approval {
                    tool_call_id: format!("t{turn}"),
                    decision: ApprovalDecision::Once,
                    source: ApprovalSource::Mode,
                },
            });
            s.steps.push_back(step_t {
                delay: Duration::from_millis(500),
                event: SessionEvent::ToolResult {
                    turn,
                    tool_call_id: format!("t{turn}"),
                    tool: "read_file".into(),
                    ok: true,
                    output: "fn main() {\n    let args = parse();\n    run(args);\n}\n\nfn parse() -> args_t {\n    args_t::default()\n}\n(preview fixture text: no file was read)".into(),
                    truncated: false,
                    simulated: true,
                },
            });
            reply_steps(
                s,
                turn,
                1,
                "`main` parses the arguments and hands them to `run`. Click the tool block, or select it with **j/k** and press **enter**, to see the whole result. *(Scripted preview text.)*",
                true,
                &[],
            );
            end_turn_steps(s, turn, 2);
        }
        2 => {
            let call = ReplyItem::Call {
                call: parsed_call_t {
                    tool: "replace_lines".into(),
                    args: Vec::new(),
                },
            };
            reply_steps(
                s,
                turn,
                0,
                "I will rename the greeting in one small edit.",
                true,
                &[call],
            );
            s.steps.push_back(step_t {
                delay: Duration::from_millis(120),
                event: SessionEvent::ToolCall {
                    turn,
                    tool_call_id: format!("t{turn}"),
                    tool: "replace_lines".into(),
                    class: ApprovalClass::Write,
                    args: vec![
                        tool_arg_t {
                            name: "path".into(),
                            value: "src/greeting.rs".into(),
                        },
                        tool_arg_t {
                            name: "lines".into(),
                            value: "12-14".into(),
                        },
                    ],
                },
            });
            s.steps.push_back(step_t {
                delay: Duration::from_millis(100),
                event: SessionEvent::ApprovalRequested {
                    turn,
                    tool_call_id: format!("t{turn}"),
                    tool: "replace_lines".into(),
                    class: ApprovalClass::Write,
                    summary: "src/greeting.rs, lines 12-14:\n-    println!(\"hello\");\n+    println!(\"hello, ncagent\");".into(),
                    reason: "write tools ask in the ask approval mode".into(),
                },
            });
            s.next_call = 1;
        }
        3 => {
            let ask = ReplyItem::Ask {
                question: harness::event::question_t {
                    question: "Which part should I explain?".into(),
                    options: vec![
                        "the level strip".into(),
                        "the dissolve".into(),
                        "the sidebar".into(),
                    ],
                },
            };
            reply_steps(s, turn, 0, "Happy to explain one part.", true, &[ask]);
            s.steps.push_back(step_t {
                delay: Duration::from_millis(120),
                event: SessionEvent::QuestionAsked {
                    turn,
                    question_id: format!("q{turn}"),
                    question: "Which part should I explain?".into(),
                    options: vec![
                        "the level strip".into(),
                        "the dissolve".into(),
                        "the sidebar".into(),
                    ],
                },
            });
            s.next_call = 1;
        }
        4 => {
            s.steps.push_back(step_t {
                delay: Duration::from_millis(250),
                event: call_start(turn, 0, 0),
            });
            level_steps(s, turn, 0, 0, "", false);
            s.steps.push_back(step_t {
                delay: Duration::from_millis(80),
                event: SessionEvent::CallEnd {
                    turn,
                    call: 0,
                    attempt: 0,
                    text: "@call read_file\npath: src/main.rs".into(),
                    truncated: false,
                    valid: false,
                    issues: vec!["the @call block has no @end line".into()],
                    items: Vec::new(),
                    passes: LevelLens.len() as u32,
                },
            });
            s.steps.push_back(step_t {
                delay: Duration::from_millis(250),
                event: call_start(turn, 0, 1),
            });
            let text = "The first sample did not parse, so the call was sampled again with the next seed. This level run had no drafts, so the blocks followed each level's length instead. *(Scripted preview text.)*";
            level_steps(s, turn, 0, 1, text, false);
            s.steps.push_back(step_t {
                delay: Duration::from_millis(80),
                event: SessionEvent::CallEnd {
                    turn,
                    call: 0,
                    attempt: 1,
                    text: text.into(),
                    truncated: false,
                    valid: true,
                    issues: Vec::new(),
                    items: vec![ReplyItem::Text { text: text.into() }],
                    passes: LevelLens.len() as u32,
                },
            });
            end_turn_steps(s, turn, 1);
        }
        _ => {
            reply_steps(s, turn, 0, IntroReply, true, &[]);
            end_turn_steps(s, turn, 1);
        }
    }
}

/// The capabilities the fixture reports. They mirror the demo `standard` profile in
/// PROJECT.md ("Demo backend") and describe no released checkpoint.
pub fn preview_capabilities() -> capabilities_t {
    capabilities_t {
        prefix_window: 4096,
        reply_len: 1024,
        level_count: LevelLens.len() as u32,
        level_lens: LevelLens.to_vec(),
        eos_id: Some(256),
        pad_id: 257,
        vocab_size: 258,
        cut_at_eos: true,
        supports_drafts: true,
        default_sampler: sampler_t::greedy(),
    }
}

pub fn preview_info(cwd: &str) -> runtime_info_t {
    let project = cwd
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();
    runtime_info_t {
        version: env!("CARGO_PKG_VERSION").to_string(),
        identity: model_identity_t {
            id: "ui-preview".into(),
            kind: BackendKind::Demo,
            export_hash: None,
            note: format!("{PreviewLabel}: a UI fixture, not a model"),
        },
        capabilities: preview_capabilities(),
        cwd: cwd.to_string(),
        project,
        approval_mode: ApprovalMode::Ask,
        notice: Some(format!(
            "{PreviewLabel}. Every reply, tool result and timing here is fixed fixture text; nothing runs and no model is called."
        )),
        state_note: Some("preview state is kept in memory only".into()),
        hints: vec![
            hint_t {
                trigger: "tool".into(),
                description: "a read tool call with an expandable result".into(),
            },
            hint_t {
                trigger: "approve".into(),
                description: "a write that waits for your approval".into(),
            },
            hint_t {
                trigger: "ask".into(),
                description: "a question with numbered options".into(),
            },
            hint_t {
                trigger: "retry".into(),
                description: "an invalid sample, then a resample without drafts".into(),
            },
        ],
    }
}

struct stored_t {
    id: String,
    events: Vec<stamped_event_t>,
}

/// The runtime side of the fixture.
pub struct scripted_runtime_t {
    info: runtime_info_t,
    mode: Mode,
    handles: Mutex<Vec<script_handle_t>>,
    stored: Mutex<Vec<stored_t>>,
    /// Folders, kept in memory with the real rules of `harness::organize` (D50, D51).
    organization: Mutex<organization_t>,
    /// When set, `organization` and `organize` return this error, as a store whose file does
    /// not load would (D52).
    organization_error: Mutex<Option<SessionError>>,
    /// Settings and custom themes, kept with the harness's in-memory store, so its rules apply.
    settings: memory_settings_store_t,
    /// When set, `settings` and `set_settings` return this error, as a settings file that does
    /// not load would (D70).
    settings_error: Mutex<Option<SessionError>>,
    /// When set, `delete_session` returns this error and changes nothing.
    delete_error: Mutex<Option<SessionError>>,
    /// Every id `delete_session` was asked to delete, oldest first.
    deleted: Mutex<Vec<String>>,
    /// Every operation `organize` applied, oldest first.
    applied: Mutex<Vec<OrganizeOp>>,
    /// Theme files `read_theme_file` finds, by path, with what reading each gives. Any other
    /// path is refused by the in-memory store, which reads no files.
    theme_files: Mutex<Vec<(String, Result<custom_theme_t, SessionError>)>>,
    /// Theme files `themes` says it skipped.
    skipped_themes: Mutex<Vec<String>>,
}

impl scripted_runtime_t {
    /// A runtime for tests: sessions emit only what the test pushes.
    pub fn manual(info: runtime_info_t) -> Self {
        scripted_runtime_t {
            info,
            mode: Mode::Manual,
            handles: Mutex::new(Vec::new()),
            stored: Mutex::new(Vec::new()),
            organization: Mutex::new(organization_t::new()),
            organization_error: Mutex::new(None),
            settings: memory_settings_store_t::new(),
            settings_error: Mutex::new(None),
            delete_error: Mutex::new(None),
            deleted: Mutex::new(Vec::new()),
            applied: Mutex::new(Vec::new()),
            theme_files: Mutex::new(Vec::new()),
            skipped_themes: Mutex::new(Vec::new()),
        }
    }

    /// A runtime for the gallery: sessions play canned scenarios on a timer.
    pub fn preview(info: runtime_info_t) -> Self {
        scripted_runtime_t {
            info,
            mode: Mode::Preview,
            handles: Mutex::new(Vec::new()),
            stored: Mutex::new(Vec::new()),
            organization: Mutex::new(organization_t::new()),
            organization_error: Mutex::new(None),
            settings: memory_settings_store_t::new(),
            settings_error: Mutex::new(None),
            delete_error: Mutex::new(None),
            deleted: Mutex::new(Vec::new()),
            applied: Mutex::new(Vec::new()),
            theme_files: Mutex::new(Vec::new()),
            skipped_themes: Mutex::new(Vec::new()),
        }
    }

    /// Add a saved session that `open_session` replays.
    pub fn store(&self, id: &str, events: Vec<stamped_event_t>) {
        if let Ok(mut s) = self.stored.lock() {
            s.push(stored_t {
                id: id.to_string(),
                events,
            });
        }
    }

    /// Make every folder call fail with `err`, or work again with `None`.
    pub fn fail_organization(&self, err: Option<SessionError>) {
        *self
            .organization_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = err;
    }

    /// Apply a folder change directly, for test and gallery setup.
    pub fn seed_organization(&self, ops: &[OrganizeOp]) -> Result<(), SessionError> {
        for op in ops {
            self.organize(op.clone())?;
        }
        Ok(())
    }

    fn organization_failure(&self) -> Option<SessionError> {
        self.organization_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Make `settings` and `set_settings` fail with `err`, or work again with `None`.
    pub fn fail_settings(&self, err: Option<SessionError>) {
        *self
            .settings_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = err;
    }

    /// Make `delete_session` fail with `err`, or work again with `None`.
    pub fn fail_delete(&self, err: Option<SessionError>) {
        *self
            .delete_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = err;
    }

    /// Save settings directly, for test and gallery setup. The store's rules apply.
    pub fn seed_settings(&self, settings: &settings_t) -> Result<(), SessionError> {
        Ok(self.settings.save(settings)?)
    }

    /// Make `read_theme_file(path)` give `result`, as a file at that path would.
    pub fn offer_theme_file(&self, path: &str, result: Result<custom_theme_t, SessionError>) {
        self.theme_files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((path.to_string(), result));
    }

    /// Make `themes` report these files as skipped.
    pub fn skip_theme_files(&self, files: Vec<String>) {
        *self
            .skipped_themes
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = files;
    }

    /// Save a custom theme directly, for test and gallery setup. The store's rules apply.
    pub fn seed_theme(&self, theme: &custom_theme_t) -> Result<(), SessionError> {
        Ok(self.settings.save_theme(theme)?)
    }

    /// The settings as saved now, whatever `fail_settings` says.
    pub fn saved_settings(&self) -> Result<settings_t, SessionError> {
        Ok(self.settings.load()?)
    }

    /// Every id `delete_session` was asked to delete, oldest first.
    pub fn deleted(&self) -> Vec<String> {
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every operation `organize` applied, oldest first.
    pub fn applied(&self) -> Vec<OrganizeOp> {
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// True when a saved session with this id is held.
    pub fn is_stored(&self, id: &str) -> bool {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|s| s.id == id)
    }

    fn settings_failure(&self) -> Option<SessionError> {
        self.settings_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The handle of the n-th session made by `new_session` or `open_session`.
    pub fn handle(&self, index: usize) -> Option<script_handle_t> {
        self.handles.lock().ok().and_then(|h| h.get(index).cloned())
    }

    fn make(&self, id: String, first: Vec<SessionEvent>, autoplay: bool) -> scripted_session_t {
        let mut shared = shared_t {
            incoming: first.into(),
            calls: Vec::new(),
            phase: SessionPhase::Idle,
            queue: Vec::new(),
            mode: self.info.approval_mode,
            seq: 0,
            steps: VecDeque::new(),
            due: None,
            wait: Wait::None,
            turn: 0,
            next_call: 0,
            levels_done: 0,
        };
        if autoplay {
            start_turn(&mut shared, "Show me around.".into(), false);
        }
        let shared = Arc::new(Mutex::new(shared));
        if let Ok(mut h) = self.handles.lock() {
            h.push(script_handle_t {
                id: id.clone(),
                shared: Arc::clone(&shared),
            });
        }
        scripted_session_t {
            id,
            mode: self.mode,
            shared,
        }
    }

    fn session_start(&self, id: &str) -> SessionEvent {
        let caps = &self.info.capabilities;
        SessionEvent::SessionStart {
            session_id: id.to_string(),
            simulated: true,
            label: Some(PreviewLabel.to_string()),
            model_id: self.info.identity.id.clone(),
            backend_kind: self.info.identity.kind,
            contract_version: harness::backend::ContractVersion,
            approval_mode: self.info.approval_mode,
            cwd: self.info.cwd.clone(),
            project: self.info.project.clone(),
            prefix_window: caps.prefix_window,
            reply_len: caps.reply_len,
            level_count: caps.level_count,
            level_lens: caps.level_lens.clone(),
            session_seed: 4242,
        }
    }
}

impl runtime_t for scripted_runtime_t {
    fn info(&self) -> &runtime_info_t {
        &self.info
    }

    fn list_sessions(&self) -> Result<Vec<session_summary_t>, SessionError> {
        let stored = self.stored.lock().map_err(|_| SessionError::Internal {
            message: "fixture lock".into(),
        })?;
        let mut out: Vec<session_summary_t> = stored
            .iter()
            .filter_map(|s| summarize_events(&s.id, &s.events))
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.updated_ms));
        Ok(out)
    }

    fn new_session(&self) -> Result<Box<dyn session_t>, SessionError> {
        let n = self.handles.lock().map(|h| h.len()).unwrap_or(0);
        let id = format!("preview-{}", n + 1);
        let start = self.session_start(&id);
        let autoplay = self.mode == Mode::Preview && n == 0;
        Ok(Box::new(self.make(id, vec![start], autoplay)))
    }

    fn open_session(&self, id: &str) -> Result<opened_session_t, SessionError> {
        let events = {
            let stored = self.stored.lock().map_err(|_| SessionError::Internal {
                message: "fixture lock".into(),
            })?;
            stored
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.events.clone())
                .ok_or_else(|| SessionError::NotFound { id: id.to_string() })?
        };
        let session = self.make(id.to_string(), Vec::new(), false);
        if let Ok(mut s) = session.shared.lock() {
            s.seq = events.len() as u64;
        }
        Ok(opened_session_t {
            session: Box::new(session),
            events,
            skipped_lines: 0,
        })
    }

    fn organization(&self) -> Result<organization_t, SessionError> {
        if let Some(err) = self.organization_failure() {
            return Err(err);
        }
        Ok(self
            .organization
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn organize(&self, op: OrganizeOp) -> Result<organization_t, SessionError> {
        if let Some(err) = self.organization_failure() {
            return Err(err);
        }
        let mut held = self
            .organization
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = held.clone();
        next.apply(&op)?;
        *held = next.clone();
        self.applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(op);
        Ok(next)
    }

    /// Removes the saved events, then forgets the filing and settings, as the real runtime does
    /// (D59, D67). A session with nothing saved is still forgotten.
    fn delete_session(&self, id: &str) -> Result<organization_t, SessionError> {
        self.deleted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id.to_string());
        if let Some(err) = self
            .delete_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            return Err(err);
        }
        if let Some(err) = self.organization_failure() {
            return Err(err);
        }
        let mut held = self
            .organization
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut next = held.clone();
        next.apply(&OrganizeOp::ForgetSession {
            session_id: id.to_string(),
        })?;
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|s| s.id != id);
        *held = next.clone();
        Ok(next)
    }

    fn settings(&self) -> Result<settings_t, SessionError> {
        if let Some(err) = self.settings_failure() {
            return Err(err);
        }
        Ok(self.settings.load()?)
    }

    fn set_settings(&self, settings: settings_t) -> Result<settings_t, SessionError> {
        if let Some(err) = self.settings_failure() {
            return Err(err);
        }
        self.settings.save(&settings)?;
        Ok(settings)
    }

    fn themes(&self) -> Result<loaded_themes_t, SessionError> {
        let mut loaded = self.settings.list_themes()?;
        loaded.skipped = self
            .skipped_themes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(loaded)
    }

    fn read_theme_file(&self, path: &str) -> Result<custom_theme_t, SessionError> {
        let offered = self
            .theme_files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, r)| r.clone());
        match offered {
            Some(result) => result,
            None => Ok(self.settings.read_theme_file(std::path::Path::new(path))?),
        }
    }

    fn save_theme(&self, theme: custom_theme_t) -> Result<loaded_themes_t, SessionError> {
        self.settings.save_theme(&theme)?;
        Ok(self.settings.list_themes()?)
    }

    fn remove_theme(&self, id: &str) -> Result<loaded_themes_t, SessionError> {
        self.settings.remove_theme(id)?;
        Ok(self.settings.list_themes()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coarse_drafts_merge_words() {
        let text = "a b c d e f g h";
        assert_eq!(coarse_draft(text, 0, 4), "abcdefgh");
        assert_eq!(coarse_draft(text, 1, 4), "abcd efgh");
        assert_eq!(coarse_draft(text, 2, 4), "ab cd ef gh");
        assert_eq!(coarse_draft(text, 3, 4), text);
    }

    #[test]
    fn manual_session_records_calls_and_tracks_phase() {
        let rt = scripted_runtime_t::manual(preview_info("/tmp/p"));
        let mut s = rt.new_session().expect("session");
        let h = rt.handle(0).expect("handle");
        let first = s.pump();
        assert!(matches!(first[0].event, SessionEvent::SessionStart { .. }));
        h.push([SessionEvent::TurnStart { turn: 1 }]);
        s.pump();
        assert_eq!(s.phase(), SessionPhase::Generating);
        s.submit("later".into());
        assert_eq!(s.queued(), vec!["later".to_string()]);
        assert_eq!(
            s.set_approval_mode(ApprovalMode::Full),
            Err(SessionError::FullRequiresFlag)
        );
        assert_eq!(
            h.calls(),
            vec![
                ScriptCall::Submit("later".into()),
                ScriptCall::SetApprovalMode(ApprovalMode::Full)
            ]
        );
    }
}
