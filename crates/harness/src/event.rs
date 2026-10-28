/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The typed event stream between the session controller and every consumer (PROJECT.md, "UI
//! prototype (P1)", "Event stream and session API").
//!
//! The UI folds these events into a transcript. A resumed session replays the same events
//! through the same fold. Each variant maps to an event name of PROJECT.md, "Sessions, events
//! and persistence", where one exists. A saved line is one JSON object: `{"seq":..,"at_ms":..,
//! "event":{"event":"user_message",...}}`.

use ncm::backend::{BackendKind, sampler_t};
use serde::{Deserialize, Serialize};

use crate::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
use crate::tools::{ApprovalClass, ToolErrorKind, parsed_call_t, tool_arg_t};

/// A question block of a reply (D13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct question_t {
    pub question: String,
    pub options: Vec<String>,
}

/// One item of a parsed reply (D13).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "item", rename_all = "snake_case")]
pub enum ReplyItem {
    Text { text: String },
    Call { call: parsed_call_t },
    Ask { question: question_t },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelScope {
    /// A call was stopped between level passes.
    Call,
    /// A running tool was stopped.
    Tool,
    /// A pending approval request was dropped.
    Approval,
    /// A pending question was dropped.
    Question,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorScope {
    Backend,
    Reply,
    Budget,
    Store,
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warning,
}

/// One event of a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    /// The first event of every session. `simulated` and `label` carry the demo marking (O1).
    SessionStart {
        session_id: String,
        simulated: bool,
        label: Option<String>,
        model_id: String,
        backend_kind: BackendKind,
        contract_version: u32,
        approval_mode: ApprovalMode,
        cwd: String,
        project: String,
        prefix_window: u32,
        reply_len: u32,
        level_count: u32,
        level_lens: Vec<u32>,
        session_seed: u64,
    },
    /// A message entered the transcript. `from_queue` is true when it waited behind a turn.
    UserMessage {
        turn: u32,
        text: String,
        from_queue: bool,
    },
    /// A message typed during a turn was queued.
    MessageQueued {
        text: String,
    },
    TurnStart {
        turn: u32,
    },
    /// A call to the backend started. `attempt` is 0 for the first sample of a call.
    CallStart {
        turn: u32,
        call: u32,
        attempt: u32,
        seed: u64,
        prompt_hash: String,
        packed_tokens: u32,
        window: u32,
        dropped_turns: u32,
        sampler: sampler_t,
    },
    /// One level pass finished.
    LevelDone {
        turn: u32,
        call: u32,
        attempt: u32,
        level: u32,
        level_count: u32,
        level_len: u32,
        elapsed_ms: u64,
        draft: Option<String>,
    },
    /// A call returned its final block, already cut and parsed.
    CallEnd {
        turn: u32,
        call: u32,
        attempt: u32,
        text: String,
        truncated: bool,
        valid: bool,
        issues: Vec<String>,
        items: Vec<ReplyItem>,
        passes: u32,
    },
    /// A valid reply asked for a tool.
    ToolCall {
        turn: u32,
        tool_call_id: String,
        tool: String,
        class: ApprovalClass,
        args: Vec<tool_arg_t>,
    },
    /// The policy asked the user. `summary` is the exact command or the planned change.
    ApprovalRequested {
        turn: u32,
        tool_call_id: String,
        tool: String,
        class: ApprovalClass,
        summary: String,
        reason: String,
    },
    /// A call was allowed or refused, by the user, the mode or a session grant.
    Approval {
        tool_call_id: String,
        decision: ApprovalDecision,
        source: ApprovalSource,
    },
    ToolResult {
        turn: u32,
        tool_call_id: String,
        tool: String,
        ok: bool,
        output: String,
        truncated: bool,
        simulated: bool,
    },
    ToolError {
        turn: u32,
        tool_call_id: String,
        tool: String,
        kind: ToolErrorKind,
        message: String,
    },
    QuestionAsked {
        turn: u32,
        question_id: String,
        question: String,
        options: Vec<String>,
    },
    AskAnswer {
        turn: u32,
        question_id: String,
        answer: String,
        option_index: Option<u32>,
    },
    ApprovalModeChanged {
        mode: ApprovalMode,
    },
    Cancel {
        turn: u32,
        call: Option<u32>,
        levels_done: Option<u32>,
        scope: CancelScope,
    },
    /// An error with a stable `name`. It never ends the program.
    Error {
        turn: Option<u32>,
        scope: ErrorScope,
        name: String,
        message: String,
    },
    Notice {
        level: NoticeLevel,
        text: String,
    },
    TurnEnd {
        turn: u32,
        outcome: TurnOutcome,
        calls: u32,
        passes: u32,
    },
}

/// An event with its place in the session and its time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct stamped_event_t {
    /// Starts at 0 and rises by 1 for each event of the session.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub event: SessionEvent,
}
