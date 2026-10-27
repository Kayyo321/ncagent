/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The transcript model: a pure fold of `stamped_event_t` into blocks.
//!
//! Live events and replayed events go through the same [`transcript_t::fold`]. The only
//! difference is the optional animation clock, which records when a level or a final block
//! arrived so the effect can sweep; a replayed event has no clock and renders settled. Odd or
//! out-of-order sequences never panic: a missing block is created where it is first needed.

use std::collections::HashMap;

use harness::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
use harness::backend::BackendKind;
use harness::event::{
    CancelScope, ErrorScope, NoticeLevel, ReplyItem, SessionEvent, TurnOutcome, stamped_event_t,
};
use harness::tools::{ApprovalClass, ToolErrorKind, tool_arg_t};

/// What `session_start` says about the session.
#[derive(Clone, Debug, PartialEq)]
pub struct session_meta_t {
    pub session_id: String,
    pub simulated: bool,
    pub label: Option<String>,
    pub model_id: String,
    pub backend_kind: BackendKind,
    pub contract_version: u32,
    pub cwd: String,
    pub project: String,
    pub prefix_window: u32,
    pub reply_len: u32,
    pub level_count: u32,
    pub level_lens: Vec<u32>,
    pub session_seed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct level_view_t {
    pub level: u32,
    pub level_count: u32,
    pub level_len: u32,
    pub elapsed_ms: u64,
    pub draft: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct call_end_view_t {
    pub text: String,
    /// The text items of a valid reply, joined; the raw text of an invalid one.
    pub display: String,
    pub truncated: bool,
    pub valid: bool,
    pub issues: Vec<String>,
    /// Tools the reply asked for, in order.
    pub calls: Vec<String>,
    pub asks: usize,
    pub passes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptStop {
    Cancelled {
        levels_done: Option<u32>,
    },
    Failed {
        name: String,
    },
    /// The turn ended before this attempt reported an end.
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct attempt_view_t {
    pub attempt: u32,
    pub seed: u64,
    pub packed_tokens: u32,
    pub window: u32,
    pub dropped_turns: u32,
    pub level_count: u32,
    pub levels: Vec<level_view_t>,
    pub end: Option<call_end_view_t>,
    pub stop: Option<AttemptStop>,
}

impl attempt_view_t {
    pub fn running(&self) -> bool {
        self.end.is_none() && self.stop.is_none()
    }

    /// Sum of the measured pass times reported by the backend.
    pub fn elapsed_ms(&self) -> u64 {
        self.levels
            .iter()
            .fold(0u64, |acc, l| acc.saturating_add(l.elapsed_ms))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct reply_view_t {
    pub turn: u32,
    pub call: u32,
    pub attempts: Vec<attempt_view_t>,
}

impl reply_view_t {
    pub fn last(&self) -> Option<&attempt_view_t> {
        self.attempts.last()
    }

    pub fn running(&self) -> bool {
        self.last().is_some_and(attempt_view_t::running)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    AwaitingApproval,
    Running,
    Ok,
    Failed,
    Denied,
    Cancelled,
}

impl ToolStatus {
    pub fn name(self) -> &'static str {
        match self {
            ToolStatus::Pending => "pending",
            ToolStatus::AwaitingApproval => "awaiting approval",
            ToolStatus::Running => "running",
            ToolStatus::Ok => "ok",
            ToolStatus::Failed => "failed",
            ToolStatus::Denied => "denied",
            ToolStatus::Cancelled => "cancelled",
        }
    }

    pub fn finished(self) -> bool {
        matches!(
            self,
            ToolStatus::Ok | ToolStatus::Failed | ToolStatus::Denied | ToolStatus::Cancelled
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct approval_req_t {
    pub summary: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct tool_output_t {
    pub ok: bool,
    pub text: String,
    pub truncated: bool,
    pub simulated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct tool_view_t {
    pub tool_call_id: String,
    pub tool: String,
    pub class: ApprovalClass,
    pub args: Vec<tool_arg_t>,
    pub status: ToolStatus,
    pub approval: Option<approval_req_t>,
    pub decision: Option<(ApprovalDecision, ApprovalSource)>,
    pub output: Option<tool_output_t>,
    pub error: Option<(ToolErrorKind, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct question_view_t {
    pub question_id: String,
    pub question: String,
    pub options: Vec<String>,
    pub answer: Option<(String, Option<u32>)>,
    pub dropped: bool,
}

impl question_view_t {
    pub fn pending(&self) -> bool {
        self.answer.is_none() && !self.dropped
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    /// The title card.
    Banner,
    User {
        text: String,
        from_queue: bool,
    },
    Reply(reply_view_t),
    Tool(tool_view_t),
    Question(question_view_t),
    Cancel {
        scope: CancelScope,
        call: Option<u32>,
        levels_done: Option<u32>,
    },
    Error {
        scope: ErrorScope,
        name: String,
        message: String,
    },
    Notice {
        level: NoticeLevel,
        text: String,
    },
    ModeChange {
        mode: ApprovalMode,
    },
    TurnEnd {
        outcome: TurnOutcome,
        calls: u32,
        passes: u32,
        elapsed_ms: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct block_t {
    pub id: u64,
    /// Rises on every change, so a layout cache keyed by it stays correct.
    pub revision: u64,
    pub turn: Option<u32>,
    pub kind: BlockKind,
    pub expanded: bool,
}

impl block_t {
    pub fn expandable(&self) -> bool {
        match &self.kind {
            BlockKind::Tool(_) => true,
            BlockKind::Reply(r) => r
                .attempts
                .iter()
                .any(|a| a.end.as_ref().is_some_and(|e| !e.valid)),
            _ => false,
        }
    }
}

/// When the parts of an attempt arrived, on the app clock. Only live events set these.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct anim_track_t {
    pub started_ms: Option<u64>,
    pub levels_ms: Vec<Option<u64>>,
    pub ended_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct context_view_t {
    pub packed_tokens: u32,
    pub window: u32,
    pub dropped_turns: u32,
}

/// Counters of the turn in progress, or of the last one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct turn_view_t {
    pub turn: u32,
    pub open: bool,
    pub calls: u32,
    pub passes: u32,
    pub started_at_ms: u64,
    pub outcome: Option<TurnOutcome>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct transcript_t {
    pub blocks: Vec<block_t>,
    pub meta: Option<session_meta_t>,
    pub turn: turn_view_t,
    pub context: Option<context_view_t>,
    pub mode: Option<ApprovalMode>,
    pub last_seq: Option<u64>,
    pub last_at_ms: u64,
    /// Animation arrivals per (block id, attempt). Not part of the folded meaning.
    pub anim: HashMap<(u64, u32), anim_track_t>,
    next_id: u64,
}

impl Default for transcript_t {
    fn default() -> Self {
        Self::new()
    }
}

impl transcript_t {
    pub fn new() -> Self {
        let mut t = transcript_t {
            blocks: Vec::new(),
            meta: None,
            turn: turn_view_t::default(),
            context: None,
            mode: None,
            last_seq: None,
            last_at_ms: 0,
            anim: HashMap::new(),
            next_id: 0,
        };
        t.push(None, BlockKind::Banner);
        t
    }

    fn push(&mut self, turn: Option<u32>, kind: BlockKind) -> usize {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.blocks.push(block_t {
            id,
            revision: 0,
            turn,
            kind,
            expanded: false,
        });
        self.blocks.len() - 1
    }

    fn touch(&mut self, index: usize) {
        if let Some(b) = self.blocks.get_mut(index) {
            b.revision = b.revision.wrapping_add(1);
        }
    }

    pub fn index_of(&self, id: u64) -> Option<usize> {
        // Ids rise with position, so a binary search finds the block.
        self.blocks.binary_search_by_key(&id, |b| b.id).ok()
    }

    pub fn toggle_expanded(&mut self, id: u64) -> bool {
        match self.index_of(id) {
            Some(i) => {
                if let Some(b) = self.blocks.get_mut(i)
                    && b.expandable()
                {
                    b.expanded = !b.expanded;
                    b.revision = b.revision.wrapping_add(1);
                    return true;
                }
                false
            }
            None => false,
        }
    }

    fn find_reply(&self, turn: u32, call: u32) -> Option<usize> {
        self.blocks.iter().rposition(|b| match &b.kind {
            BlockKind::Reply(r) => r.turn == turn && r.call == call,
            _ => false,
        })
    }

    fn reply_index(&mut self, turn: u32, call: u32) -> usize {
        match self.find_reply(turn, call) {
            Some(i) => i,
            None => self.push(
                Some(turn),
                BlockKind::Reply(reply_view_t {
                    turn,
                    call,
                    attempts: Vec::new(),
                }),
            ),
        }
    }

    fn attempt_mut(&mut self, index: usize, attempt: u32) -> Option<&mut attempt_view_t> {
        let default_k = self.meta.as_ref().map(|m| m.level_count).unwrap_or(0);
        let block = self.blocks.get_mut(index)?;
        let BlockKind::Reply(reply) = &mut block.kind else {
            return None;
        };
        let pos = match reply.attempts.iter().position(|a| a.attempt == attempt) {
            Some(p) => p,
            None => {
                reply.attempts.push(attempt_view_t {
                    attempt,
                    seed: 0,
                    packed_tokens: 0,
                    window: 0,
                    dropped_turns: 0,
                    level_count: default_k,
                    levels: Vec::new(),
                    end: None,
                    stop: None,
                });
                reply.attempts.len() - 1
            }
        };
        reply.attempts.get_mut(pos)
    }

    fn find_tool(&self, id: &str) -> Option<usize> {
        self.blocks.iter().rposition(|b| match &b.kind {
            BlockKind::Tool(t) => t.tool_call_id == id,
            _ => false,
        })
    }

    fn tool_index(
        &mut self,
        turn: Option<u32>,
        id: &str,
        tool: &str,
        class: ApprovalClass,
    ) -> usize {
        match self.find_tool(id) {
            Some(i) => i,
            None => self.push(
                turn,
                BlockKind::Tool(tool_view_t {
                    tool_call_id: id.to_string(),
                    tool: tool.to_string(),
                    class,
                    args: Vec::new(),
                    status: ToolStatus::Pending,
                    approval: None,
                    decision: None,
                    output: None,
                    error: None,
                }),
            ),
        }
    }

    fn tool_mut(&mut self, index: usize) -> Option<&mut tool_view_t> {
        match self.blocks.get_mut(index).map(|b| &mut b.kind) {
            Some(BlockKind::Tool(t)) => Some(t),
            _ => None,
        }
    }

    /// The tool block waiting for the user's approval, if any.
    pub fn pending_approval(&self) -> Option<(usize, &tool_view_t)> {
        for (i, b) in self.blocks.iter().enumerate().rev() {
            match &b.kind {
                BlockKind::Tool(t) if t.status == ToolStatus::AwaitingApproval => {
                    return Some((i, t));
                }
                BlockKind::TurnEnd { .. } => return None,
                _ => {}
            }
        }
        None
    }

    /// The question waiting for the user's answer, if any.
    pub fn pending_question(&self) -> Option<(usize, &question_view_t)> {
        for (i, b) in self.blocks.iter().enumerate().rev() {
            match &b.kind {
                BlockKind::Question(q) if q.pending() => return Some((i, q)),
                BlockKind::TurnEnd { .. } => return None,
                _ => {}
            }
        }
        None
    }

    /// The reply block whose last attempt is still generating.
    pub fn running_reply(&self) -> Option<(usize, &reply_view_t)> {
        for (i, b) in self.blocks.iter().enumerate().rev() {
            match &b.kind {
                BlockKind::Reply(r) if r.running() => return Some((i, r)),
                BlockKind::TurnEnd { .. } => return None,
                _ => {}
            }
        }
        None
    }

    /// The latest reply block of any state.
    pub fn latest_reply(&self) -> Option<&reply_view_t> {
        self.blocks.iter().rev().find_map(|b| match &b.kind {
            BlockKind::Reply(r) if !r.attempts.is_empty() => Some(r),
            _ => None,
        })
    }

    /// The text of the latest complete, valid reply.
    pub fn last_reply_text(&self) -> Option<String> {
        self.blocks.iter().rev().find_map(|b| match &b.kind {
            BlockKind::Reply(r) => r
                .attempts
                .iter()
                .rev()
                .find_map(|a| {
                    a.end
                        .as_ref()
                        .filter(|e| e.valid && !e.display.trim().is_empty())
                })
                .map(|e| e.display.clone()),
            _ => None,
        })
    }

    /// Fold one event. `clock` is the app clock for a live event and `None` for a replay.
    pub fn fold(&mut self, stamped: &stamped_event_t, clock: Option<u64>) {
        self.last_seq = Some(stamped.seq);
        self.last_at_ms = self.last_at_ms.max(stamped.at_ms);
        match &stamped.event {
            SessionEvent::SessionStart {
                session_id,
                simulated,
                label,
                model_id,
                backend_kind,
                contract_version,
                approval_mode,
                cwd,
                project,
                prefix_window,
                reply_len,
                level_count,
                level_lens,
                session_seed,
            } => {
                self.meta = Some(session_meta_t {
                    session_id: session_id.clone(),
                    simulated: *simulated,
                    label: label.clone(),
                    model_id: model_id.clone(),
                    backend_kind: *backend_kind,
                    contract_version: *contract_version,
                    cwd: cwd.clone(),
                    project: project.clone(),
                    prefix_window: *prefix_window,
                    reply_len: *reply_len,
                    level_count: *level_count,
                    level_lens: level_lens.clone(),
                    session_seed: *session_seed,
                });
                self.mode = Some(*approval_mode);
                if let Some(i) = self
                    .blocks
                    .iter()
                    .position(|b| matches!(b.kind, BlockKind::Banner))
                {
                    self.touch(i);
                }
            }
            SessionEvent::UserMessage {
                turn,
                text,
                from_queue,
            } => {
                self.push(
                    Some(*turn),
                    BlockKind::User {
                        text: text.clone(),
                        from_queue: *from_queue,
                    },
                );
            }
            SessionEvent::MessageQueued { .. } => {}
            SessionEvent::TurnStart { turn } => {
                self.turn = turn_view_t {
                    turn: *turn,
                    open: true,
                    calls: 0,
                    passes: 0,
                    started_at_ms: stamped.at_ms,
                    outcome: None,
                };
            }
            SessionEvent::CallStart {
                turn,
                call,
                attempt,
                seed,
                packed_tokens,
                window,
                dropped_turns,
                ..
            } => {
                let index = self.reply_index(*turn, *call);
                if let Some(a) = self.attempt_mut(index, *attempt) {
                    a.seed = *seed;
                    a.packed_tokens = *packed_tokens;
                    a.window = *window;
                    a.dropped_turns = *dropped_turns;
                }
                self.touch(index);
                if *attempt == 0 {
                    self.turn.calls = self.turn.calls.saturating_add(1);
                }
                self.context = Some(context_view_t {
                    packed_tokens: *packed_tokens,
                    window: *window,
                    dropped_turns: *dropped_turns,
                });
                if let Some(c) = clock {
                    let id = self.blocks.get(index).map(|b| b.id).unwrap_or(0);
                    self.anim.entry((id, *attempt)).or_default().started_ms = Some(c);
                }
            }
            SessionEvent::LevelDone {
                turn,
                call,
                attempt,
                level,
                level_count,
                level_len,
                elapsed_ms,
                draft,
            } => {
                let index = self.reply_index(*turn, *call);
                let mut position = 0usize;
                if let Some(a) = self.attempt_mut(index, *attempt) {
                    a.level_count = *level_count;
                    a.levels.push(level_view_t {
                        level: *level,
                        level_count: *level_count,
                        level_len: *level_len,
                        elapsed_ms: *elapsed_ms,
                        draft: draft.clone(),
                    });
                    position = a.levels.len() - 1;
                }
                self.touch(index);
                self.turn.passes = self.turn.passes.saturating_add(1);
                if let Some(c) = clock {
                    let id = self.blocks.get(index).map(|b| b.id).unwrap_or(0);
                    let track = self.anim.entry((id, *attempt)).or_default();
                    if track.levels_ms.len() <= position {
                        track.levels_ms.resize(position + 1, None);
                    }
                    if let Some(slot) = track.levels_ms.get_mut(position) {
                        *slot = Some(c);
                    }
                }
            }
            SessionEvent::CallEnd {
                turn,
                call,
                attempt,
                text,
                truncated,
                valid,
                issues,
                items,
                passes,
            } => {
                let index = self.reply_index(*turn, *call);
                let display = if *valid {
                    items
                        .iter()
                        .filter_map(|item| match item {
                            ReplyItem::Text { text } => Some(text.trim_matches('\n')),
                            _ => None,
                        })
                        .filter(|t| !t.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("\n\n")
                } else {
                    text.clone()
                };
                let calls = items
                    .iter()
                    .filter_map(|item| match item {
                        ReplyItem::Call { call } => Some(call.tool.clone()),
                        _ => None,
                    })
                    .collect();
                let asks = items
                    .iter()
                    .filter(|i| matches!(i, ReplyItem::Ask { .. }))
                    .count();
                if let Some(a) = self.attempt_mut(index, *attempt) {
                    a.end = Some(call_end_view_t {
                        text: text.clone(),
                        display,
                        truncated: *truncated,
                        valid: *valid,
                        issues: issues.clone(),
                        calls,
                        asks,
                        passes: *passes,
                    });
                }
                self.touch(index);
                if let Some(c) = clock {
                    let id = self.blocks.get(index).map(|b| b.id).unwrap_or(0);
                    self.anim.entry((id, *attempt)).or_default().ended_ms = Some(c);
                }
            }
            SessionEvent::ToolCall {
                turn,
                tool_call_id,
                tool,
                class,
                args,
            } => {
                let index = self.tool_index(Some(*turn), tool_call_id, tool, *class);
                if let Some(t) = self.tool_mut(index) {
                    t.args = args.clone();
                    t.class = *class;
                }
                self.touch(index);
            }
            SessionEvent::ApprovalRequested {
                turn,
                tool_call_id,
                tool,
                class,
                summary,
                reason,
            } => {
                let index = self.tool_index(Some(*turn), tool_call_id, tool, *class);
                if let Some(t) = self.tool_mut(index) {
                    t.approval = Some(approval_req_t {
                        summary: summary.clone(),
                        reason: reason.clone(),
                    });
                    if !t.status.finished() {
                        t.status = ToolStatus::AwaitingApproval;
                    }
                }
                self.touch(index);
            }
            SessionEvent::Approval {
                tool_call_id,
                decision,
                source,
            } => {
                if let Some(index) = self.find_tool(tool_call_id) {
                    if let Some(t) = self.tool_mut(index) {
                        t.decision = Some((*decision, *source));
                        if !t.status.finished() {
                            t.status = match decision {
                                ApprovalDecision::Deny => ToolStatus::Denied,
                                _ => ToolStatus::Running,
                            };
                        }
                    }
                    self.touch(index);
                }
            }
            SessionEvent::ToolResult {
                turn,
                tool_call_id,
                tool,
                ok,
                output,
                truncated,
                simulated,
            } => {
                let index = self.tool_index(Some(*turn), tool_call_id, tool, ApprovalClass::Read);
                if let Some(t) = self.tool_mut(index) {
                    t.output = Some(tool_output_t {
                        ok: *ok,
                        text: output.clone(),
                        truncated: *truncated,
                        simulated: *simulated,
                    });
                    t.status = if *ok {
                        ToolStatus::Ok
                    } else {
                        ToolStatus::Failed
                    };
                }
                self.touch(index);
            }
            SessionEvent::ToolError {
                turn,
                tool_call_id,
                tool,
                kind,
                message,
            } => {
                let index = self.tool_index(Some(*turn), tool_call_id, tool, ApprovalClass::Read);
                if let Some(t) = self.tool_mut(index) {
                    t.error = Some((*kind, message.clone()));
                    t.status = match kind {
                        ToolErrorKind::Denied => ToolStatus::Denied,
                        ToolErrorKind::Cancelled => ToolStatus::Cancelled,
                        _ => ToolStatus::Failed,
                    };
                }
                self.touch(index);
            }
            SessionEvent::QuestionAsked {
                turn,
                question_id,
                question,
                options,
            } => {
                self.push(
                    Some(*turn),
                    BlockKind::Question(question_view_t {
                        question_id: question_id.clone(),
                        question: question.clone(),
                        options: options.clone(),
                        answer: None,
                        dropped: false,
                    }),
                );
            }
            SessionEvent::AskAnswer {
                question_id,
                answer,
                option_index,
                ..
            } => {
                let found = self.blocks.iter().rposition(|b| match &b.kind {
                    BlockKind::Question(q) => q.question_id == *question_id,
                    _ => false,
                });
                if let Some(i) = found {
                    if let Some(BlockKind::Question(q)) =
                        self.blocks.get_mut(i).map(|b| &mut b.kind)
                    {
                        q.answer = Some((answer.clone(), *option_index));
                    }
                    self.touch(i);
                }
            }
            SessionEvent::ApprovalModeChanged { mode } => {
                self.mode = Some(*mode);
                self.push(None, BlockKind::ModeChange { mode: *mode });
            }
            SessionEvent::Cancel {
                turn,
                call,
                levels_done,
                scope,
            } => {
                self.apply_cancel(*turn, *call, *levels_done, *scope);
                self.push(
                    Some(*turn),
                    BlockKind::Cancel {
                        scope: *scope,
                        call: *call,
                        levels_done: *levels_done,
                    },
                );
            }
            SessionEvent::Error {
                turn,
                scope,
                name,
                message,
            } => {
                if *scope == ErrorScope::Backend
                    && let Some((i, _)) = self.running_reply()
                {
                    if let Some(BlockKind::Reply(r)) = self.blocks.get_mut(i).map(|b| &mut b.kind)
                        && let Some(a) = r.attempts.last_mut()
                    {
                        a.stop = Some(AttemptStop::Failed { name: name.clone() });
                    }
                    self.touch(i);
                }
                self.push(
                    *turn,
                    BlockKind::Error {
                        scope: *scope,
                        name: name.clone(),
                        message: message.clone(),
                    },
                );
            }
            SessionEvent::Notice { level, text } => {
                self.push(
                    None,
                    BlockKind::Notice {
                        level: *level,
                        text: text.clone(),
                    },
                );
            }
            SessionEvent::TurnEnd {
                turn,
                outcome,
                calls,
                passes,
            } => {
                self.close_open_items();
                let elapsed_ms = if self.turn.turn == *turn && self.turn.started_at_ms > 0 {
                    Some(stamped.at_ms.saturating_sub(self.turn.started_at_ms))
                } else {
                    None
                };
                self.turn.turn = *turn;
                self.turn.open = false;
                self.turn.calls = *calls;
                self.turn.passes = *passes;
                self.turn.outcome = Some(*outcome);
                self.push(
                    Some(*turn),
                    BlockKind::TurnEnd {
                        outcome: *outcome,
                        calls: *calls,
                        passes: *passes,
                        elapsed_ms,
                    },
                );
            }
        }
    }

    fn apply_cancel(
        &mut self,
        turn: u32,
        call: Option<u32>,
        levels_done: Option<u32>,
        scope: CancelScope,
    ) {
        match scope {
            CancelScope::Call => {
                let index = match call {
                    Some(c) => self.find_reply(turn, c),
                    None => self.running_reply().map(|(i, _)| i),
                };
                if let Some(i) = index {
                    if let Some(BlockKind::Reply(r)) = self.blocks.get_mut(i).map(|b| &mut b.kind)
                        && let Some(a) = r.attempts.last_mut()
                        && a.end.is_none()
                    {
                        a.stop = Some(AttemptStop::Cancelled { levels_done });
                    }
                    self.touch(i);
                }
            }
            CancelScope::Tool | CancelScope::Approval => {
                let found = self.blocks.iter().rposition(|b| match &b.kind {
                    BlockKind::Tool(t) => !t.status.finished(),
                    _ => false,
                });
                if let Some(i) = found {
                    if let Some(t) = self.tool_mut(i) {
                        t.status = ToolStatus::Cancelled;
                    }
                    self.touch(i);
                }
            }
            CancelScope::Question => {
                if let Some((i, _)) = self.pending_question() {
                    if let Some(BlockKind::Question(q)) =
                        self.blocks.get_mut(i).map(|b| &mut b.kind)
                    {
                        q.dropped = true;
                    }
                    self.touch(i);
                }
            }
        }
    }

    /// At the end of a turn nothing can still be pending.
    fn close_open_items(&mut self) {
        let mut touched = Vec::new();
        for (i, b) in self.blocks.iter_mut().enumerate().rev() {
            match &mut b.kind {
                BlockKind::TurnEnd { .. } => break,
                BlockKind::Tool(t) if !t.status.finished() => {
                    t.status = ToolStatus::Cancelled;
                    touched.push(i);
                }
                BlockKind::Question(q) if q.pending() => {
                    q.dropped = true;
                    touched.push(i);
                }
                BlockKind::Reply(r) => {
                    if let Some(a) = r.attempts.last_mut()
                        && a.running()
                    {
                        a.stop = Some(AttemptStop::Interrupted);
                        touched.push(i);
                    }
                }
                _ => {}
            }
        }
        for i in touched {
            self.touch(i);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use harness::backend::sampler_t;
    use harness::tools::parsed_call_t;

    pub(crate) fn stamp(seq: u64, event: SessionEvent) -> stamped_event_t {
        stamped_event_t {
            seq,
            at_ms: 1_000 + seq * 100,
            event,
        }
    }

    pub(crate) fn session_start() -> SessionEvent {
        SessionEvent::SessionStart {
            session_id: "s1".into(),
            simulated: true,
            label: Some("DEMO: simulated model, not NCM output".into()),
            model_id: "demo-standard".into(),
            backend_kind: BackendKind::Demo,
            contract_version: 1,
            approval_mode: ApprovalMode::Ask,
            cwd: "/home/u/proj".into(),
            project: "proj".into(),
            prefix_window: 4096,
            reply_len: 1024,
            level_count: 3,
            level_lens: vec![8, 32, 1024],
            session_seed: 4242,
        }
    }

    pub(crate) fn call_start(turn: u32, call: u32, attempt: u32) -> SessionEvent {
        SessionEvent::CallStart {
            turn,
            call,
            attempt,
            seed: 8484 + u64::from(attempt),
            prompt_hash: "h".into(),
            packed_tokens: 1204,
            window: 4096,
            dropped_turns: 0,
            sampler: sampler_t::greedy(),
        }
    }

    pub(crate) fn level_done(
        turn: u32,
        call: u32,
        attempt: u32,
        level: u32,
        draft: Option<&str>,
    ) -> SessionEvent {
        SessionEvent::LevelDone {
            turn,
            call,
            attempt,
            level,
            level_count: 3,
            level_len: [8, 32, 1024][level.min(2) as usize],
            elapsed_ms: 400 + u64::from(level) * 10,
            draft: draft.map(str::to_string),
        }
    }

    pub(crate) fn call_end(
        turn: u32,
        call: u32,
        attempt: u32,
        text: &str,
        valid: bool,
    ) -> SessionEvent {
        SessionEvent::CallEnd {
            turn,
            call,
            attempt,
            text: text.into(),
            truncated: false,
            valid,
            issues: if valid {
                vec![]
            } else {
                vec!["unclosed @call block".into()]
            },
            items: if valid {
                vec![ReplyItem::Text { text: text.into() }]
            } else {
                vec![]
            },
            passes: 3,
        }
    }

    /// One full turn with a tool call that asks for approval, and a question.
    pub(crate) fn full_session() -> Vec<stamped_event_t> {
        let events = vec![
            session_start(),
            SessionEvent::TurnStart { turn: 1 },
            SessionEvent::UserMessage {
                turn: 1,
                text: "please approve".into(),
                from_queue: false,
            },
            call_start(1, 0, 0),
            level_done(1, 0, 0, 0, Some("I will")),
            level_done(1, 0, 0, 1, Some("I will edit it")),
            level_done(1, 0, 0, 2, Some("I will edit the file.")),
            SessionEvent::CallEnd {
                turn: 1,
                call: 0,
                attempt: 0,
                text: "I will edit the file.\n@call replace_lines".into(),
                truncated: false,
                valid: true,
                issues: vec![],
                items: vec![
                    ReplyItem::Text {
                        text: "I will edit the file.".into(),
                    },
                    ReplyItem::Call {
                        call: parsed_call_t {
                            tool: "replace_lines".into(),
                            args: vec![],
                        },
                    },
                ],
                passes: 3,
            },
            SessionEvent::ToolCall {
                turn: 1,
                tool_call_id: "t1".into(),
                tool: "replace_lines".into(),
                class: ApprovalClass::Write,
                args: vec![tool_arg_t {
                    name: "path".into(),
                    value: "src/lib.rs".into(),
                }],
            },
            SessionEvent::ApprovalRequested {
                turn: 1,
                tool_call_id: "t1".into(),
                tool: "replace_lines".into(),
                class: ApprovalClass::Write,
                summary: "replace lines 3-4 of src/lib.rs".into(),
                reason: "write tools ask in ask mode".into(),
            },
            SessionEvent::Approval {
                tool_call_id: "t1".into(),
                decision: ApprovalDecision::Once,
                source: ApprovalSource::User,
            },
            SessionEvent::ToolResult {
                turn: 1,
                tool_call_id: "t1".into(),
                tool: "replace_lines".into(),
                ok: true,
                output: "replaced 2 lines\nline two".into(),
                truncated: false,
                simulated: true,
            },
            call_start(1, 1, 0),
            level_done(1, 1, 0, 0, None),
            call_end(1, 1, 0, "broken", false),
            call_start(1, 1, 1),
            level_done(1, 1, 1, 0, None),
            SessionEvent::CallEnd {
                turn: 1,
                call: 1,
                attempt: 1,
                text: "@ask".into(),
                truncated: false,
                valid: true,
                issues: vec![],
                items: vec![],
                passes: 1,
            },
            SessionEvent::QuestionAsked {
                turn: 1,
                question_id: "q1".into(),
                question: "Which one?".into(),
                options: vec!["first".into(), "second".into()],
            },
            SessionEvent::AskAnswer {
                turn: 1,
                question_id: "q1".into(),
                answer: "second".into(),
                option_index: Some(1),
            },
            call_start(1, 2, 0),
            level_done(1, 2, 0, 0, None),
            call_end(1, 2, 0, "## Done\n\nAll **good**.", true),
            SessionEvent::Notice {
                level: NoticeLevel::Info,
                text: "a notice".into(),
            },
            SessionEvent::TurnEnd {
                turn: 1,
                outcome: TurnOutcome::Completed,
                calls: 3,
                passes: 5,
            },
        ];
        events
            .into_iter()
            .enumerate()
            .map(|(i, e)| stamp(i as u64, e))
            .collect()
    }

    fn fold_all(events: &[stamped_event_t], live: bool) -> transcript_t {
        let mut t = transcript_t::new();
        for (i, e) in events.iter().enumerate() {
            t.fold(e, live.then_some(i as u64 * 10));
        }
        t
    }

    #[test]
    fn full_session_folds_into_expected_blocks() {
        let t = fold_all(&full_session(), false);
        let kinds: Vec<&str> = t
            .blocks
            .iter()
            .map(|b| match &b.kind {
                BlockKind::Banner => "banner",
                BlockKind::User { .. } => "user",
                BlockKind::Reply(_) => "reply",
                BlockKind::Tool(_) => "tool",
                BlockKind::Question(_) => "question",
                BlockKind::Cancel { .. } => "cancel",
                BlockKind::Error { .. } => "error",
                BlockKind::Notice { .. } => "notice",
                BlockKind::ModeChange { .. } => "mode",
                BlockKind::TurnEnd { .. } => "turn_end",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "banner", "user", "reply", "tool", "reply", "question", "reply", "notice",
                "turn_end"
            ]
        );
        let BlockKind::Tool(tool) = &t.blocks[3].kind else {
            panic!("tool block")
        };
        assert_eq!(tool.status, ToolStatus::Ok);
        assert_eq!(
            tool.decision,
            Some((ApprovalDecision::Once, ApprovalSource::User))
        );
        assert!(tool.output.as_ref().is_some_and(|o| o.simulated));
        let BlockKind::Reply(retry) = &t.blocks[4].kind else {
            panic!("reply block")
        };
        assert_eq!(retry.attempts.len(), 2);
        assert!(!retry.attempts[0].end.as_ref().is_some_and(|e| e.valid));
        assert!(t.blocks[4].expandable());
        assert_eq!(t.turn.passes, 5);
        assert!(!t.turn.open);
        assert_eq!(
            t.last_reply_text().as_deref(),
            Some("## Done\n\nAll **good**.")
        );
        assert!(t.pending_approval().is_none());
        assert!(t.pending_question().is_none());
        assert_eq!(t.context.map(|c| c.packed_tokens), Some(1204));
    }

    #[test]
    fn replay_equals_live_except_animation_clock() {
        let events = full_session();
        let live = fold_all(&events, true);
        let replay = fold_all(&events, false);
        assert_eq!(live.blocks, replay.blocks);
        assert_eq!(live.turn, replay.turn);
        assert!(replay.anim.is_empty());
        assert!(!live.anim.is_empty());
    }

    #[test]
    fn approval_opens_and_closes() {
        let events = full_session();
        let mut t = transcript_t::new();
        for e in &events[..10] {
            t.fold(e, None);
        }
        let (_, pending) = t.pending_approval().expect("pending approval");
        assert_eq!(pending.tool_call_id, "t1");
        t.fold(&events[10], None);
        assert!(t.pending_approval().is_none());
    }

    #[test]
    fn cancel_and_turn_end_close_pending_items() {
        let events = full_session();
        let mut t = transcript_t::new();
        for e in &events[..10] {
            t.fold(e, None);
        }
        t.fold(
            &stamp(
                50,
                SessionEvent::Cancel {
                    turn: 1,
                    call: None,
                    levels_done: None,
                    scope: CancelScope::Approval,
                },
            ),
            None,
        );
        assert!(t.pending_approval().is_none());
        // Question dropped by turn end.
        t.fold(
            &stamp(
                51,
                SessionEvent::QuestionAsked {
                    turn: 1,
                    question_id: "q9".into(),
                    question: "?".into(),
                    options: vec![],
                },
            ),
            None,
        );
        assert!(t.pending_question().is_some());
        t.fold(
            &stamp(
                52,
                SessionEvent::TurnEnd {
                    turn: 1,
                    outcome: TurnOutcome::Cancelled,
                    calls: 1,
                    passes: 3,
                },
            ),
            None,
        );
        assert!(t.pending_question().is_none());
    }

    #[test]
    fn cancel_call_marks_running_attempt() {
        let mut t = transcript_t::new();
        t.fold(&stamp(0, call_start(1, 0, 0)), None);
        t.fold(&stamp(1, level_done(1, 0, 0, 0, None)), None);
        assert!(t.running_reply().is_some());
        t.fold(
            &stamp(
                2,
                SessionEvent::Cancel {
                    turn: 1,
                    call: Some(0),
                    levels_done: Some(1),
                    scope: CancelScope::Call,
                },
            ),
            None,
        );
        assert!(t.running_reply().is_none());
    }

    #[test]
    fn odd_sequences_do_not_panic() {
        let mut t = transcript_t::new();
        let odd = vec![
            level_done(9, 3, 2, 7, Some("x")),
            call_end(4, 4, 4, "", true),
            SessionEvent::Approval {
                tool_call_id: "nope".into(),
                decision: ApprovalDecision::Deny,
                source: ApprovalSource::User,
            },
            SessionEvent::AskAnswer {
                turn: 0,
                question_id: "none".into(),
                answer: "x".into(),
                option_index: Some(99),
            },
            SessionEvent::ToolResult {
                turn: 0,
                tool_call_id: "ghost".into(),
                tool: "read_file".into(),
                ok: false,
                output: String::new(),
                truncated: true,
                simulated: false,
            },
            SessionEvent::TurnEnd {
                turn: 77,
                outcome: TurnOutcome::Failed,
                calls: 0,
                passes: 0,
            },
            SessionEvent::Cancel {
                turn: 0,
                call: None,
                levels_done: None,
                scope: CancelScope::Question,
            },
            session_start(),
            session_start(),
        ];
        for (i, e) in odd.into_iter().enumerate() {
            t.fold(&stamp(i as u64, e), Some(0));
        }
        assert!(t.blocks.len() > 3);
    }

    #[test]
    fn long_transcripts_fold_fast() {
        let start = std::time::Instant::now();
        let mut t = transcript_t::new();
        let mut seq = 0u64;
        for turn in 1..=3000u32 {
            for e in [
                SessionEvent::TurnStart { turn },
                SessionEvent::UserMessage {
                    turn,
                    text: format!("message {turn}"),
                    from_queue: false,
                },
                call_start(turn, 0, 0),
                level_done(turn, 0, 0, 0, None),
                call_end(turn, 0, 0, "reply text", true),
                SessionEvent::TurnEnd {
                    turn,
                    outcome: TurnOutcome::Completed,
                    calls: 1,
                    passes: 1,
                },
            ] {
                t.fold(&stamp(seq, e), None);
                seq += 1;
            }
        }
        assert_eq!(t.blocks.len(), 1 + 3000 * 3);
        assert!(start.elapsed().as_secs() < 5);
    }

    #[test]
    fn toggle_only_expandable_blocks() {
        let mut t = fold_all(&full_session(), false);
        let tool_id = t.blocks[3].id;
        let rev = t.blocks[3].revision;
        assert!(t.toggle_expanded(tool_id));
        assert!(t.blocks[3].expanded);
        assert!(t.blocks[3].revision != rev);
        let user_id = t.blocks[1].id;
        assert!(!t.toggle_expanded(user_id));
        assert!(!t.toggle_expanded(9999));
    }
}
