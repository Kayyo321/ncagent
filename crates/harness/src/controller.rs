/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The session controller: turns, calls, retries, approvals, questions, cancellation and the
//! message queue (PROJECT.md, "UI prototype (P1)", "Controller rules").
//!
//! A model call and a tool run each run on one `std::thread` worker (D39). `pump` drains the
//! worker's messages with `try_recv` and never blocks. A turn runs one call or one tool at a
//! time, so at most one worker is live. Every event is stamped, written to the store and queued
//! for the next `pump`. This module holds no scenario logic: the model's reply and the tool
//! results decide what happens next.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use ncm::backend::{
    BackendError, CallOutcome, ContractVersion, backend_t, call_request_t, cancel_token_t,
    capabilities_t, completion_t, level_progress_t,
};

use crate::api::{SessionError, SessionPhase, session_t};
use crate::approval::{
    ApprovalDecision, ApprovalMode, ApprovalSource, Verdict, approval_policy_t, summary_for,
};
use crate::event::{
    CancelScope, ErrorScope, NoticeLevel, ReplyItem, SessionEvent, TurnOutcome, stamped_event_t,
};
use crate::pack::{TranscriptEntry, pack_prefix};
use crate::replay::restored_t;
use crate::reply::parse_reply;
use crate::runtime::{project_name, runtime_config_t};
use crate::store::session_store_t;
use crate::tools::{
    self, ToolErrorKind, tool_call_t, tool_failure_t, tool_outcome_t, tool_runner_t,
};

/// The message of a tool error when the user refuses a call.
const DeniedMessage: &str = "denied by user";

/// What every session of one runtime shares.
#[derive(Clone)]
pub struct session_deps_t {
    pub backend: Arc<dyn backend_t>,
    pub runner: Arc<dyn tool_runner_t>,
    pub store: Arc<dyn session_store_t>,
    pub config: runtime_config_t,
}

/// What a worker sends back. A call worker ends with `CallDone`, a tool worker with `ToolDone`.
enum WorkerMsg {
    Level(level_progress_t),
    CallDone(Result<CallOutcome, BackendError>),
    ToolDone(Result<tool_outcome_t, tool_failure_t>),
}

/// The receiving end of the one worker that is running.
struct worker_t {
    rx: Receiver<WorkerMsg>,
}

/// One session: a transcript, a queue and the state of the turn in progress.
pub struct controller_session_t {
    id: String,
    deps: session_deps_t,
    caps: capabilities_t,
    policy: approval_policy_t,
    session_seed: u64,
    phase: SessionPhase,
    /// The sequence number of the next event.
    seq: u64,
    store_failed: bool,
    /// Events not yet returned by `pump`.
    outbox: Vec<stamped_event_t>,
    transcript: Vec<TranscriptEntry>,
    queue: VecDeque<String>,
    /// The last turn that started.
    turn: u32,
    turn_calls: u32,
    turn_passes: u32,
    /// The number the next call gets.
    next_call: u32,
    /// The call in progress, and its attempt and the levels reported so far in that attempt.
    cur_call: u32,
    cur_attempt: u32,
    cur_levels: u32,
    /// The cancel token of the running call or tool.
    cancel: Option<cancel_token_t>,
    worker: Option<worker_t>,
    /// The items of the valid reply not yet walked.
    items: VecDeque<ReplyItem>,
    /// Action items (calls and questions) of the current reply walked so far.
    action_k: u32,
    /// True once an action item of the current reply has run. Then the next call follows.
    ran_action: bool,
    pending_approval: Option<tool_call_t>,
    pending_question: Option<String>,
    running_tool: Option<tool_call_t>,
}

impl Drop for controller_session_t {
    fn drop(&mut self) {
        // The running call or tool stops at its next check. Its worker then sends into a closed
        // channel, and that send is ignored.
        if let Some(token) = &self.cancel {
            token.cancel();
        }
    }
}

impl controller_session_t {
    /// A session with no events yet, in phase `Idle`.
    fn fresh(deps: session_deps_t, id: String) -> Self {
        let caps = deps.backend.capabilities().clone();
        let policy = approval_policy_t::new(deps.config.approval, deps.config.cwd.clone());
        let session_seed = deps.config.session_seed;
        controller_session_t {
            id,
            deps,
            caps,
            policy,
            session_seed,
            phase: SessionPhase::Idle,
            seq: 0,
            store_failed: false,
            outbox: Vec::new(),
            transcript: Vec::new(),
            queue: VecDeque::new(),
            turn: 0,
            turn_calls: 0,
            turn_passes: 0,
            next_call: 0,
            cur_call: 0,
            cur_attempt: 0,
            cur_levels: 0,
            cancel: None,
            worker: None,
            items: VecDeque::new(),
            action_k: 0,
            ran_action: false,
            pending_approval: None,
            pending_question: None,
            running_tool: None,
        }
    }

    /// A new session. Its first event is `session_start`.
    pub(crate) fn start(deps: session_deps_t, id: String) -> Self {
        let mut session = Self::fresh(deps, id);
        let event = session_start_event(
            &session.deps,
            &session.id,
            &session.caps,
            session.session_seed,
            session.policy.mode(),
        );
        session.emit(event);
        session
    }

    /// A session rebuilt from its stored events. It continues the sequence, the turn and call
    /// numbers, and the transcript. A turn that the last run did not finish is closed here as
    /// failed, with a notice, so the log stays consistent.
    pub(crate) fn resume(deps: session_deps_t, id: String, restored: restored_t) -> Self {
        let mut session = Self::fresh(deps, id);
        session.seq = restored.next_seq;
        session.turn = restored.turn;
        session.next_call = restored.call_index;
        session.session_seed = restored.session_seed;
        session.transcript = restored.entries;
        // Session grants are not stored, so a resumed session asks again for every path.
        if restored.approval_mode == ApprovalMode::Full
            && session.deps.config.approval != ApprovalMode::Full
        {
            let mode = session.deps.config.approval;
            session.policy.set_mode(mode);
            session.emit(SessionEvent::ApprovalModeChanged { mode });
            session.emit(SessionEvent::Notice {
                level: NoticeLevel::Warning,
                text: "approval mode full is not restored: it needs the command-line flag, so the session resumes in the mode of this run".to_string(),
            });
        } else {
            session.policy.set_mode(restored.approval_mode);
        }
        if restored.mid_turn {
            session.emit(SessionEvent::Notice {
                level: NoticeLevel::Warning,
                text: "the previous run ended in the middle of a turn".to_string(),
            });
            session.emit(SessionEvent::TurnEnd {
                turn: session.turn,
                outcome: TurnOutcome::Failed,
                calls: 0,
                passes: 0,
            });
        }
        session
    }

    /// The events written while this session was created or reopened, which `pump` has not
    /// returned yet.
    pub(crate) fn take_events(&mut self) -> Vec<stamped_event_t> {
        std::mem::take(&mut self.outbox)
    }

    /// Gives the next sequence number and the time to an event, without storing it.
    fn stamp(&mut self, event: SessionEvent) -> stamped_event_t {
        let stamped = stamped_event_t {
            seq: self.seq,
            at_ms: (self.deps.config.clock)(),
            event,
        };
        self.seq = self.seq.saturating_add(1);
        stamped
    }

    /// Stores an event and queues it for `pump`. A store failure is reported once, as an error
    /// event. The session keeps working, and later writes are skipped.
    fn emit(&mut self, event: SessionEvent) {
        let stamped = self.stamp(event);
        let failure = if self.store_failed {
            None
        } else {
            self.deps.store.append(&self.id, &stamped).err()
        };
        self.outbox.push(stamped);
        if let Some(err) = failure {
            self.store_failed = true;
            let turn = (self.turn > 0).then_some(self.turn);
            let report = self.stamp(SessionEvent::Error {
                turn,
                scope: ErrorScope::Store,
                name: "store_write_failed".to_string(),
                message: format!("the session log could not be written: {err}"),
            });
            self.outbox.push(report);
        }
    }

    /// Starts the oldest queued message as a new turn, when the session is idle. Returns false
    /// when it started nothing.
    fn start_queued_turn(&mut self) -> bool {
        if self.phase != SessionPhase::Idle {
            return false;
        }
        match self.queue.pop_front() {
            Some(text) => {
                self.begin_turn(text, true);
                true
            }
            None => false,
        }
    }

    /// Starts queued turns while the session is idle. A turn that ends at once, such as one
    /// that exhausts the pass budget, lets the next one start, so this loops rather than
    /// recursing.
    fn kick(&mut self) {
        while self.start_queued_turn() {}
    }

    fn begin_turn(&mut self, text: String, from_queue: bool) {
        self.turn = self.turn.saturating_add(1);
        self.turn_calls = 0;
        self.turn_passes = 0;
        let turn = self.turn;
        self.emit(SessionEvent::UserMessage {
            turn,
            text: text.clone(),
            from_queue,
        });
        self.transcript.push(TranscriptEntry::User { text });
        self.emit(SessionEvent::TurnStart { turn });
        self.start_call(0);
    }

    /// Moves every queued message into the transcript, before the first attempt of a call.
    fn move_queue_into_transcript(&mut self) {
        let turn = self.turn;
        while let Some(text) = self.queue.pop_front() {
            self.emit(SessionEvent::UserMessage {
                turn,
                text: text.clone(),
                from_queue: true,
            });
            self.transcript.push(TranscriptEntry::User { text });
        }
    }

    /// Starts attempt `attempt` of the current call, or a new call when `attempt` is 0.
    fn start_call(&mut self, attempt: u32) {
        let budget = self.deps.config.pass_budget;
        if self.turn_passes >= budget {
            let message = format!(
                "the turn used {} level passes and the pass budget is {budget}",
                self.turn_passes
            );
            self.emit(SessionEvent::Error {
                turn: Some(self.turn),
                scope: ErrorScope::Budget,
                name: "pass_budget_exhausted".to_string(),
                message,
            });
            self.end_turn(TurnOutcome::Failed);
            return;
        }
        if attempt == 0 {
            self.cur_call = self.next_call;
            self.next_call = self.next_call.saturating_add(1);
            self.turn_calls = self.turn_calls.saturating_add(1);
            self.move_queue_into_transcript();
        }
        self.cur_attempt = attempt;
        self.cur_levels = 0;
        let packed = pack_prefix(self.deps.backend.codec(), &self.caps, &self.transcript);
        let seed = call_seed(self.session_seed, self.cur_call).wrapping_add(u64::from(attempt));
        let token = cancel_token_t::new();
        let event = SessionEvent::CallStart {
            turn: self.turn,
            call: self.cur_call,
            attempt,
            seed,
            prompt_hash: packed.hash.clone(),
            packed_tokens: u32::try_from(packed.ids.len()).unwrap_or(u32::MAX),
            window: self.caps.prefix_window,
            dropped_turns: u32::try_from(packed.dropped_entries).unwrap_or(u32::MAX),
            sampler: self.caps.default_sampler,
        };
        self.emit(event);
        let request = call_request_t {
            prefix: packed.ids,
            sampler: self.caps.default_sampler,
            seed,
            cancel: token.clone(),
        };
        match spawn_call(Arc::clone(&self.deps.backend), request) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.cancel = Some(token);
                self.phase = SessionPhase::Generating;
            }
            Err(err) => self.fail_spawn(err),
        }
    }

    fn fail_spawn(&mut self, err: io::Error) {
        self.emit(SessionEvent::Error {
            turn: Some(self.turn),
            scope: ErrorScope::Internal,
            name: "worker_spawn_failed".to_string(),
            message: format!("could not start a worker thread: {err}"),
        });
        self.end_turn(TurnOutcome::Failed);
    }

    /// Ends the turn: emits `turn_end` and clears everything that belonged to it. The caller
    /// starts the next queued turn, if any.
    fn end_turn(&mut self, outcome: TurnOutcome) {
        self.emit(SessionEvent::TurnEnd {
            turn: self.turn,
            outcome,
            calls: self.turn_calls,
            passes: self.turn_passes,
        });
        self.phase = SessionPhase::Idle;
        self.items.clear();
        self.action_k = 0;
        self.ran_action = false;
        self.pending_approval = None;
        self.pending_question = None;
        self.running_tool = None;
        self.cancel = None;
        self.worker = None;
    }

    fn request_stop(&mut self) {
        if let Some(token) = &self.cancel {
            token.cancel();
        }
    }

    /// Drains the running worker's messages until none is left, or until the worker is gone.
    /// A message can start another worker, and the loop then drains that one too.
    fn drain_worker(&mut self) {
        loop {
            let received = match self.worker.as_ref() {
                Some(worker) => worker.rx.try_recv(),
                None => return,
            };
            match received {
                Ok(WorkerMsg::Level(progress)) => self.on_level(progress),
                Ok(WorkerMsg::CallDone(result)) => {
                    self.worker = None;
                    self.on_call_done(result);
                }
                Ok(WorkerMsg::ToolDone(result)) => {
                    self.worker = None;
                    self.on_tool_done(result);
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The worker ended without its last message: it panicked.
                    self.worker = None;
                    self.emit(SessionEvent::Error {
                        turn: Some(self.turn),
                        scope: ErrorScope::Internal,
                        name: "worker_lost".to_string(),
                        message: "the worker stopped without returning a result".to_string(),
                    });
                    self.end_turn(TurnOutcome::Failed);
                }
            }
        }
    }

    fn on_level(&mut self, progress: level_progress_t) {
        self.cur_levels = self.cur_levels.saturating_add(1);
        self.turn_passes = self.turn_passes.saturating_add(1);
        let elapsed_ms = u64::try_from(progress.elapsed.as_millis()).unwrap_or(u64::MAX);
        let event = SessionEvent::LevelDone {
            turn: self.turn,
            call: self.cur_call,
            attempt: self.cur_attempt,
            level: progress.level,
            level_count: progress.level_count,
            level_len: progress.level_len,
            elapsed_ms,
            draft: progress.draft,
        };
        self.emit(event);
    }

    fn on_call_done(&mut self, result: Result<CallOutcome, BackendError>) {
        match result {
            Ok(CallOutcome::Completed(completion)) => self.on_completed(completion),
            Ok(CallOutcome::Cancelled(cancelled)) => {
                self.emit(SessionEvent::Cancel {
                    turn: self.turn,
                    call: Some(self.cur_call),
                    levels_done: Some(cancelled.levels_done),
                    scope: CancelScope::Call,
                });
                self.end_turn(TurnOutcome::Cancelled);
            }
            Err(err) => {
                self.emit(SessionEvent::Error {
                    turn: Some(self.turn),
                    scope: ErrorScope::Backend,
                    name: err.name().to_string(),
                    message: err.to_string(),
                });
                self.end_turn(TurnOutcome::Failed);
            }
        }
    }

    /// A call returned its final block. The block is cut, decoded and parsed. A call that
    /// races with a cancel is still recorded, but no further call or tool starts.
    fn on_completed(&mut self, completion: completion_t) {
        if let Err(err) = completion.validate_for(&self.caps) {
            self.emit(SessionEvent::Error {
                turn: Some(self.turn),
                scope: ErrorScope::Backend,
                name: err.name().to_string(),
                message: err.to_string(),
            });
            self.end_turn(TurnOutcome::Failed);
            return;
        }
        let (kept, truncated) = cut_reply(completion.tokens, &self.caps);
        let text = self.deps.backend.codec().decode(&kept);
        let parsed = parse_reply(&text);
        let valid = parsed.valid();
        let cancelled = self.phase == SessionPhase::Cancelling;
        let turn = self.turn;
        let call = self.cur_call;
        let attempt = self.cur_attempt;
        let passes = self.cur_levels;
        self.emit(SessionEvent::CallEnd {
            turn,
            call,
            attempt,
            text: text.clone(),
            truncated,
            valid,
            issues: parsed.issues.clone(),
            items: parsed.items.clone(),
            passes,
        });
        if truncated {
            let message = format!(
                "reply truncated at {} positions; continuation of truncated replies (D11) is not implemented in this prototype",
                self.caps.reply_len
            );
            self.emit(SessionEvent::Notice {
                level: NoticeLevel::Warning,
                text: message,
            });
        }
        if !valid && !cancelled {
            let max_attempts = self.deps.config.max_attempts;
            if attempt.saturating_add(1) < max_attempts {
                self.start_call(attempt.saturating_add(1));
                return;
            }
            let first = parsed
                .issues
                .first()
                .cloned()
                .unwrap_or_else(|| "the reply did not parse".to_string());
            self.emit(SessionEvent::Error {
                turn: Some(turn),
                scope: ErrorScope::Reply,
                name: "invalid_reply".to_string(),
                message: format!("no valid reply after {max_attempts} attempts: {first}"),
            });
            self.end_turn(TurnOutcome::Failed);
            return;
        }
        if valid {
            self.transcript.push(TranscriptEntry::Assistant { text });
        }
        if cancelled {
            self.emit(SessionEvent::Cancel {
                turn,
                call: Some(call),
                levels_done: Some(passes),
                scope: CancelScope::Call,
            });
            self.end_turn(TurnOutcome::Cancelled);
            return;
        }
        self.items = VecDeque::from(parsed.items);
        self.action_k = 0;
        self.ran_action = false;
        self.walk_items();
    }

    /// Walks the items of the valid reply in order, until one waits for the user or the tool
    /// runs. When the reply is done, the next call follows if an action ran, and otherwise the
    /// turn completes.
    fn walk_items(&mut self) {
        while let Some(item) = self.items.pop_front() {
            let k = self.action_k;
            let turn = self.turn;
            let call_no = self.cur_call;
            match item {
                ReplyItem::Text { .. } => {}
                ReplyItem::Call { call } => {
                    self.action_k = self.action_k.saturating_add(1);
                    self.ran_action = true;
                    let id = action_id("t", turn, call_no, k);
                    let Some(spec) = tools::find(&call.tool) else {
                        let message = format!("no tool named {}", call.tool);
                        self.record_tool_error(id, call.tool, ToolErrorKind::Invalid, message);
                        continue;
                    };
                    let tool_call = tool_call_t {
                        id: id.clone(),
                        tool: call.tool,
                        args: call.args,
                    };
                    self.emit(SessionEvent::ToolCall {
                        turn,
                        tool_call_id: id.clone(),
                        tool: tool_call.tool.clone(),
                        class: spec.class,
                        args: tool_call.args.clone(),
                    });
                    match self.policy.evaluate(spec, &tool_call) {
                        Verdict::Allow { source } => {
                            self.emit(SessionEvent::Approval {
                                tool_call_id: id,
                                decision: ApprovalDecision::Once,
                                source,
                            });
                            self.run_tool(tool_call);
                            return;
                        }
                        Verdict::Ask { reason } => {
                            self.emit(SessionEvent::ApprovalRequested {
                                turn,
                                tool_call_id: id,
                                tool: tool_call.tool.clone(),
                                class: spec.class,
                                summary: summary_for(&tool_call),
                                reason,
                            });
                            self.pending_approval = Some(tool_call);
                            self.phase = SessionPhase::AwaitingApproval;
                            return;
                        }
                    }
                }
                ReplyItem::Ask { question } => {
                    self.action_k = self.action_k.saturating_add(1);
                    self.ran_action = true;
                    let id = action_id("q", turn, call_no, k);
                    self.emit(SessionEvent::QuestionAsked {
                        turn,
                        question_id: id.clone(),
                        question: question.question,
                        options: question.options,
                    });
                    self.pending_question = Some(id);
                    self.phase = SessionPhase::AwaitingAnswer;
                    return;
                }
            }
        }
        if self.ran_action {
            self.start_call(0);
        } else {
            self.end_turn(TurnOutcome::Completed);
        }
    }

    /// Records a tool error as an event and as a transcript entry, the way replay reads it.
    fn record_tool_error(
        &mut self,
        tool_call_id: String,
        tool: String,
        kind: ToolErrorKind,
        message: String,
    ) {
        self.emit(SessionEvent::ToolError {
            turn: self.turn,
            tool_call_id,
            tool: tool.clone(),
            kind,
            message: message.clone(),
        });
        self.transcript
            .push(TranscriptEntry::ToolError { tool, message });
    }

    fn run_tool(&mut self, call: tool_call_t) {
        let token = cancel_token_t::new();
        match spawn_tool(Arc::clone(&self.deps.runner), call.clone(), token.clone()) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.cancel = Some(token);
                self.running_tool = Some(call);
                self.phase = SessionPhase::RunningTool;
            }
            Err(err) => self.fail_spawn(err),
        }
    }

    fn on_tool_done(&mut self, result: Result<tool_outcome_t, tool_failure_t>) {
        let Some(call) = self.running_tool.take() else {
            self.emit(SessionEvent::Error {
                turn: Some(self.turn),
                scope: ErrorScope::Internal,
                name: "no_running_tool".to_string(),
                message: "a tool finished but none was recorded as running".to_string(),
            });
            self.end_turn(TurnOutcome::Failed);
            return;
        };
        let turn = self.turn;
        match result {
            Ok(outcome) => {
                self.emit(SessionEvent::ToolResult {
                    turn,
                    tool_call_id: call.id.clone(),
                    tool: call.tool.clone(),
                    ok: outcome.ok,
                    output: outcome.output.clone(),
                    truncated: outcome.truncated,
                    simulated: outcome.simulated,
                });
                self.transcript.push(TranscriptEntry::ToolResult {
                    tool: call.tool.clone(),
                    text: outcome.output,
                });
            }
            Err(failure) => {
                self.record_tool_error(
                    call.id.clone(),
                    call.tool.clone(),
                    failure.kind,
                    failure.message,
                );
            }
        }
        if self.phase == SessionPhase::Cancelling {
            self.emit(SessionEvent::Cancel {
                turn,
                call: Some(self.cur_call),
                levels_done: None,
                scope: CancelScope::Tool,
            });
            self.end_turn(TurnOutcome::Cancelled);
            return;
        }
        self.walk_items();
    }
}

impl session_t for controller_session_t {
    fn id(&self) -> &str {
        &self.id
    }

    fn submit(&mut self, text: String) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let message = trimmed.to_string();
        if self.phase == SessionPhase::Idle && self.queue.is_empty() {
            self.begin_turn(message, false);
        } else {
            self.emit(SessionEvent::MessageQueued {
                text: message.clone(),
            });
            self.queue.push_back(message);
        }
        self.kick();
    }

    fn cancel(&mut self) {
        match self.phase {
            SessionPhase::Idle | SessionPhase::Cancelling => {}
            SessionPhase::Generating | SessionPhase::RunningTool => {
                self.request_stop();
                self.phase = SessionPhase::Cancelling;
            }
            SessionPhase::AwaitingApproval => {
                let turn = self.turn;
                self.emit(SessionEvent::Cancel {
                    turn,
                    call: None,
                    levels_done: None,
                    scope: CancelScope::Approval,
                });
                self.end_turn(TurnOutcome::Cancelled);
            }
            SessionPhase::AwaitingAnswer => {
                let turn = self.turn;
                self.emit(SessionEvent::Cancel {
                    turn,
                    call: None,
                    levels_done: None,
                    scope: CancelScope::Question,
                });
                self.end_turn(TurnOutcome::Cancelled);
            }
        }
        self.kick();
    }

    fn resolve_approval(&mut self, tool_call_id: &str, decision: ApprovalDecision) {
        let waiting_for_this = self.phase == SessionPhase::AwaitingApproval
            && self
                .pending_approval
                .as_ref()
                .is_some_and(|call| call.id == tool_call_id);
        if !waiting_for_this {
            return;
        }
        let Some(call) = self.pending_approval.take() else {
            return;
        };
        match decision {
            ApprovalDecision::Deny => {
                self.emit(SessionEvent::Approval {
                    tool_call_id: call.id.clone(),
                    decision,
                    source: ApprovalSource::User,
                });
                self.record_tool_error(
                    call.id.clone(),
                    call.tool.clone(),
                    ToolErrorKind::Denied,
                    DeniedMessage.to_string(),
                );
                self.walk_items();
            }
            ApprovalDecision::Once | ApprovalDecision::Session => {
                self.policy.record(decision, &call);
                self.emit(SessionEvent::Approval {
                    tool_call_id: call.id.clone(),
                    decision,
                    source: ApprovalSource::User,
                });
                self.run_tool(call);
            }
        }
        self.kick();
    }

    fn answer_question(&mut self, question_id: &str, answer: String, option_index: Option<u32>) {
        let waiting_for_this = self.phase == SessionPhase::AwaitingAnswer
            && self.pending_question.as_deref() == Some(question_id);
        if !waiting_for_this {
            return;
        }
        self.pending_question = None;
        let turn = self.turn;
        self.emit(SessionEvent::AskAnswer {
            turn,
            question_id: question_id.to_string(),
            answer: answer.clone(),
            option_index,
        });
        self.transcript
            .push(TranscriptEntry::UserAnswer { text: answer });
        self.walk_items();
        self.kick();
    }

    fn queued(&self) -> Vec<String> {
        self.queue.iter().cloned().collect()
    }

    fn clear_queue(&mut self) {
        self.queue.clear();
    }

    fn set_approval_mode(&mut self, mode: ApprovalMode) -> Result<(), SessionError> {
        if mode == ApprovalMode::Full && self.deps.config.approval != ApprovalMode::Full {
            return Err(SessionError::FullRequiresFlag);
        }
        self.policy.set_mode(mode);
        self.emit(SessionEvent::ApprovalModeChanged { mode });
        Ok(())
    }

    fn approval_mode(&self) -> ApprovalMode {
        self.policy.mode()
    }

    fn phase(&self) -> SessionPhase {
        self.phase
    }

    fn pump(&mut self) -> Vec<stamped_event_t> {
        loop {
            self.drain_worker();
            if !self.start_queued_turn() {
                break;
            }
        }
        std::mem::take(&mut self.outbox)
    }
}

/// The seed of a call. It is always even (D42).
fn call_seed(session_seed: u64, call: u32) -> u64 {
    session_seed
        .wrapping_mul(2)
        .wrapping_add(1024u64.wrapping_mul(u64::from(call)))
}

/// The id of an action item: `t<turn>.<call>.<k>` for a tool call, `q...` for a question.
fn action_id(kind: &str, turn: u32, call: u32, k: u32) -> String {
    format!("{kind}{turn}.{call}.{k}")
}

/// Cuts the reply at its first EOS when the model asks for it. Returns the kept ids and whether
/// the reply is truncated: it has an EOS id and no EOS in its `L` positions.
fn cut_reply(tokens: Vec<u32>, caps: &capabilities_t) -> (Vec<u32>, bool) {
    let eos_at = caps
        .eos_id
        .and_then(|eos| tokens.iter().position(|id| *id == eos));
    let truncated = caps.eos_id.is_some() && eos_at.is_none();
    let kept = match eos_at {
        Some(at) if caps.cut_at_eos => tokens.into_iter().take(at).collect(),
        _ => tokens,
    };
    (kept, truncated)
}

fn session_start_event(
    deps: &session_deps_t,
    id: &str,
    caps: &capabilities_t,
    session_seed: u64,
    mode: ApprovalMode,
) -> SessionEvent {
    let identity = deps.backend.identity();
    SessionEvent::SessionStart {
        session_id: id.to_string(),
        simulated: identity.simulated(),
        label: deps.config.label.clone(),
        model_id: identity.id.clone(),
        backend_kind: identity.kind,
        contract_version: ContractVersion,
        approval_mode: mode,
        cwd: deps.config.cwd.to_string_lossy().into_owned(),
        project: project_name(&deps.config.cwd),
        prefix_window: caps.prefix_window,
        reply_len: caps.reply_len,
        level_count: caps.level_count,
        level_lens: caps.level_lens.clone(),
        session_seed,
    }
}

/// Starts a call worker. It sends one `Level` per pass and one `CallDone` at the end.
fn spawn_call(backend: Arc<dyn backend_t>, request: call_request_t) -> io::Result<worker_t> {
    let (tx, rx) = mpsc::channel::<WorkerMsg>();
    let level_tx = tx.clone();
    let _handle = thread::Builder::new()
        .name("ncagent-call".to_string())
        .spawn(move || {
            let mut on_level = |progress: level_progress_t| {
                let _ = level_tx.send(WorkerMsg::Level(progress));
            };
            let outcome = backend.run_call(request, &mut on_level);
            let _ = tx.send(WorkerMsg::CallDone(outcome));
        })?;
    Ok(worker_t { rx })
}

/// Starts a tool worker. It sends one `ToolDone` when the tool returns.
fn spawn_tool(
    runner: Arc<dyn tool_runner_t>,
    call: tool_call_t,
    cancel: cancel_token_t,
) -> io::Result<worker_t> {
    let (tx, rx) = mpsc::channel::<WorkerMsg>();
    let _handle = thread::Builder::new()
        .name("ncagent-tool".to_string())
        .spawn(move || {
            let result = runner.run(&call, &cancel);
            let _ = tx.send(WorkerMsg::ToolDone(result));
        })?;
    Ok(worker_t { rx })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::thread;
    use std::time::{Duration, Instant};

    use ncm::backend::{
        BackendError, BackendKind, CallOutcome, backend_t, call_request_t, cancel_token_t,
        cancelled_t, capabilities_t, completion_t, level_progress_t, model_identity_t, sampler_t,
        text_codec_t,
    };

    use crate::api::{SessionPhase, session_t};
    use crate::approval::ApprovalMode;
    use crate::event::{SessionEvent, stamped_event_t};
    use crate::runtime::{
        DefaultSeed, MaxAttempts, PassBudget, controller_runtime_t, runtime_config_t,
    };
    use crate::store::{
        StoreError, loaded_session_t, memory_store_t, session_store_t, session_summary_t,
    };
    use crate::tools::{ToolErrorKind, tool_call_t, tool_failure_t, tool_outcome_t, tool_runner_t};

    /// The length of every reply in positions. Long enough for the call blocks the tests use.
    pub const TestReplyLen: u32 = 256;
    pub const TestEos: u32 = 256;
    pub const TestPad: u32 = 257;
    pub const TestVocab: u32 = 258;
    /// Three level passes per call, the last one covering all `TestReplyLen` positions.
    const TestLevelLens: [u32; 3] = [32, 128, 256];

    pub fn test_clock() -> u64 {
        1_000
    }

    pub fn test_caps() -> capabilities_t {
        capabilities_t {
            prefix_window: 4096,
            reply_len: TestReplyLen,
            level_count: 3,
            level_lens: TestLevelLens.to_vec(),
            eos_id: Some(TestEos),
            pad_id: TestPad,
            vocab_size: TestVocab,
            cut_at_eos: true,
            supports_drafts: false,
            default_sampler: sampler_t::greedy(),
        }
    }

    pub fn test_config(approval: ApprovalMode) -> runtime_config_t {
        runtime_config_t {
            cwd: PathBuf::from("/srv/ncagent-test/project"),
            approval,
            session_seed: DefaultSeed,
            max_attempts: MaxAttempts,
            pass_budget: PassBudget,
            version: "test".to_string(),
            notice: None,
            label: Some("test session".to_string()),
            state_note: None,
            hints: Vec::new(),
            clock: test_clock,
        }
    }

    pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The byte codec of D41a: ids 0 to 255 are bytes, 256 is EOS and 257 is PAD.
    pub struct byte_codec_t;

    impl text_codec_t for byte_codec_t {
        fn encode(&self, text: &str) -> Vec<u32> {
            text.bytes().map(u32::from).collect()
        }

        fn decode(&self, ids: &[u32]) -> String {
            bytes_to_text(ids)
        }
    }

    /// The text of ids, with EOS, PAD and every other non-byte id dropped.
    pub fn bytes_to_text(ids: &[u32]) -> String {
        let bytes: Vec<u8> = ids.iter().filter_map(|id| u8::try_from(*id).ok()).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The `L` ids of a reply: its bytes, then EOS and PAD.
    pub fn reply_tokens(text: &str) -> Vec<u32> {
        let len = TestReplyLen as usize;
        let mut tokens: Vec<u32> = text.bytes().map(u32::from).collect();
        tokens.truncate(len);
        if tokens.len() < len {
            tokens.push(TestEos);
        }
        tokens.resize(len, TestPad);
        tokens
    }

    /// What the fake backend does on one call.
    #[derive(Clone, Debug, Default)]
    pub struct fake_plan_t {
        /// The `L` ids of the final block. `None` gives the reply "Done.".
        pub tokens: Option<Vec<u32>>,
        /// Fail with `BackendError::Failed` once this many level passes have reported.
        pub fail_after_passes: Option<u32>,
        /// Wait before this level until the call's token is cancelled.
        pub block_at_level: Option<u32>,
        /// Panic inside the worker.
        pub panic_call: bool,
    }

    impl fake_plan_t {
        pub fn reply(text: &str) -> Self {
            Self {
                tokens: Some(reply_tokens(text)),
                ..Self::default()
            }
        }
    }

    /// One call as the fake backend received it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct seen_call_t {
        pub seed: u64,
        pub prefix: Vec<u32>,
    }

    /// A backend that plays `plans` in order, one per call, and then "Done.".
    pub struct fake_backend_t {
        identity: model_identity_t,
        caps: capabilities_t,
        codec: byte_codec_t,
        plans: Mutex<VecDeque<fake_plan_t>>,
        seen: Mutex<Vec<seen_call_t>>,
    }

    impl fake_backend_t {
        pub fn new(plans: Vec<fake_plan_t>) -> Self {
            Self::with_capabilities(plans, test_caps())
        }

        pub fn with_capabilities(plans: Vec<fake_plan_t>, caps: capabilities_t) -> Self {
            fake_backend_t {
                identity: model_identity_t {
                    id: "test-model".to_string(),
                    kind: BackendKind::Demo,
                    export_hash: None,
                    note: "scripted test backend".to_string(),
                },
                caps,
                codec: byte_codec_t,
                plans: Mutex::new(VecDeque::from(plans)),
                seen: Mutex::new(Vec::new()),
            }
        }

        pub fn seen(&self) -> Vec<seen_call_t> {
            lock(&self.seen).clone()
        }
    }

    /// Waits until the token is cancelled, for at most five seconds.
    pub fn wait_for_cancel(token: &cancel_token_t) {
        let started = Instant::now();
        while !token.is_cancelled() && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(1));
        }
    }

    impl backend_t for fake_backend_t {
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
            request.validate_for(&self.caps)?;
            lock(&self.seen).push(seen_call_t {
                seed: request.seed,
                prefix: request.prefix.clone(),
            });
            let plan = lock(&self.plans)
                .pop_front()
                .unwrap_or_else(|| fake_plan_t::reply("Done."));
            if plan.panic_call {
                panic!("scripted worker panic");
            }
            for (index, len) in self.caps.level_lens.iter().enumerate() {
                let level = u32::try_from(index).unwrap_or(u32::MAX);
                if plan.block_at_level == Some(level) {
                    wait_for_cancel(&request.cancel);
                }
                if request.cancel.is_cancelled() {
                    return Ok(CallOutcome::Cancelled(cancelled_t { levels_done: level }));
                }
                on_level(level_progress_t {
                    level,
                    level_count: self.caps.level_count,
                    level_len: *len,
                    codes: vec![0; *len as usize],
                    elapsed: Duration::from_millis(1),
                    draft: None,
                });
                if plan.fail_after_passes == Some(level + 1) {
                    return Err(BackendError::Failed {
                        message: "scripted failure".to_string(),
                    });
                }
            }
            let tokens = plan.tokens.unwrap_or_else(|| reply_tokens("Done."));
            Ok(CallOutcome::Completed(completion_t { tokens }))
        }
    }

    /// A tool runner that records each call. `block` makes every call wait for cancellation.
    pub struct fake_tools_t {
        pub block: bool,
        ran: Mutex<Vec<tool_call_t>>,
    }

    impl fake_tools_t {
        pub fn new(block: bool) -> Self {
            fake_tools_t {
                block,
                ran: Mutex::new(Vec::new()),
            }
        }

        pub fn ran(&self) -> Vec<tool_call_t> {
            lock(&self.ran).clone()
        }
    }

    impl tool_runner_t for fake_tools_t {
        fn run(
            &self,
            call: &tool_call_t,
            cancel: &cancel_token_t,
        ) -> Result<tool_outcome_t, tool_failure_t> {
            lock(&self.ran).push(call.clone());
            if self.block {
                wait_for_cancel(cancel);
                return Err(tool_failure_t {
                    kind: ToolErrorKind::Cancelled,
                    message: "stopped while running".to_string(),
                });
            }
            if call
                .arg("path")
                .is_some_and(|path| path.contains("missing"))
            {
                return Err(tool_failure_t {
                    kind: ToolErrorKind::Failed,
                    message: "no such file".to_string(),
                });
            }
            Ok(tool_outcome_t {
                ok: true,
                output: format!("{} finished", call.tool),
                truncated: false,
                simulated: true,
            })
        }
    }

    /// A store whose every write fails.
    pub struct failing_store_t;

    impl session_store_t for failing_store_t {
        fn list(&self) -> Result<Vec<session_summary_t>, StoreError> {
            Ok(Vec::new())
        }

        fn load(&self, id: &str) -> Result<loaded_session_t, StoreError> {
            Err(StoreError::NotFound { id: id.to_string() })
        }

        fn append(&self, id: &str, _event: &stamped_event_t) -> Result<(), StoreError> {
            Err(StoreError::Io {
                path: id.to_string(),
                message: "disk full".to_string(),
            })
        }

        fn delete(&self, id: &str) -> Result<(), StoreError> {
            Err(StoreError::Io {
                path: id.to_string(),
                message: "disk full".to_string(),
            })
        }
    }

    pub fn runtime_with(
        backend: Arc<fake_backend_t>,
        tools: Arc<fake_tools_t>,
        store: Arc<memory_store_t>,
        config: runtime_config_t,
    ) -> controller_runtime_t {
        let backend: Arc<dyn backend_t> = backend;
        let tools: Arc<dyn tool_runner_t> = tools;
        let store: Arc<dyn session_store_t> = store;
        controller_runtime_t::new(backend, tools, store, config).expect("the test runtime is valid")
    }

    pub fn event_name(event: &SessionEvent) -> &'static str {
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

    pub fn names(events: &[stamped_event_t]) -> Vec<&'static str> {
        events
            .iter()
            .map(|stamped| event_name(&stamped.event))
            .collect()
    }

    /// Pumps until a batch holds an event that `stop` accepts, and returns everything pumped.
    pub fn pump_until(
        session: &mut dyn session_t,
        mut stop: impl FnMut(&SessionEvent) -> bool,
    ) -> Vec<stamped_event_t> {
        let started = Instant::now();
        let mut all = Vec::new();
        loop {
            let batch = session.pump();
            let done = batch.iter().any(|stamped| stop(&stamped.event));
            all.extend(batch);
            if done {
                return all;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "timed out waiting for an event, saw {:?}",
                names(&all)
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Pumps until the session is idle, and returns everything pumped.
    pub fn pump_idle(session: &mut dyn session_t) -> Vec<stamped_event_t> {
        let started = Instant::now();
        let mut all = Vec::new();
        loop {
            all.extend(session.pump());
            if session.phase() == SessionPhase::Idle {
                return all;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "timed out waiting for the session to go idle"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// The id of the first approval request in `events`.
    pub fn requested_id(events: &[stamped_event_t]) -> String {
        events
            .iter()
            .find_map(|stamped| match &stamped.event {
                SessionEvent::ApprovalRequested { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .expect("an approval was requested")
    }

    /// The id of the first question in `events`.
    pub fn question_id(events: &[stamped_event_t]) -> String {
        events
            .iter()
            .find_map(|stamped| match &stamped.event {
                SessionEvent::QuestionAsked { question_id, .. } => Some(question_id.clone()),
                _ => None,
            })
            .expect("a question was asked")
    }

    /// The first event that `pick` maps to a value.
    pub fn find_event<T>(
        events: &[stamped_event_t],
        pick: impl Fn(&SessionEvent) -> Option<T>,
    ) -> Option<T> {
        events.iter().find_map(|stamped| pick(&stamped.event))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::test_support::*;
    use crate::api::{SessionError, SessionPhase, runtime_t};
    use crate::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
    use crate::event::{
        CancelScope, ErrorScope, NoticeLevel, SessionEvent, TurnOutcome, stamped_event_t,
    };
    use crate::runtime::controller_runtime_t;
    use crate::store::{memory_store_t, session_store_t, valid_session_id};
    use crate::tools::{ApprovalClass, ToolErrorKind};

    /// The first `turn_end` of a batch.
    fn turn_end_of(events: &[stamped_event_t]) -> (TurnOutcome, u32, u32) {
        find_event(events, |event| match event {
            SessionEvent::TurnEnd {
                outcome,
                calls,
                passes,
                ..
            } => Some((*outcome, *calls, *passes)),
            _ => None,
        })
        .expect("a turn_end was emitted")
    }

    fn error_named(events: &[stamped_event_t], wanted: &str) -> Option<(ErrorScope, String)> {
        find_event(events, |event| match event {
            SessionEvent::Error { scope, name, .. } if name == wanted => {
                Some((*scope, name.clone()))
            }
            _ => None,
        })
    }

    fn runtime(
        backend: &Arc<fake_backend_t>,
        tools: &Arc<fake_tools_t>,
        approval: ApprovalMode,
    ) -> (controller_runtime_t, Arc<memory_store_t>) {
        let store = Arc::new(memory_store_t::new());
        let runtime = runtime_with(
            Arc::clone(backend),
            Arc::clone(tools),
            Arc::clone(&store),
            test_config(approval),
        );
        (runtime, store)
    }

    fn decoded(backend: &fake_backend_t, call: usize) -> String {
        let seen = backend.seen();
        let prefix = &seen.get(call).expect("the backend saw this call").prefix;
        bytes_to_text(prefix)
    }

    #[test]
    fn plain_turn_emits_events_in_order_and_counts_passes() {
        let backend = Arc::new(fake_backend_t::new(vec![fake_plan_t::reply(
            "Hello there.",
        )]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("  hi  ".to_string());
        let events = pump_idle(session.as_mut());
        assert_eq!(
            names(&events),
            vec![
                "session_start",
                "user_message",
                "turn_start",
                "call_start",
                "level_done",
                "level_done",
                "level_done",
                "call_end",
                "turn_end",
            ]
        );
        for (index, stamped) in events.iter().enumerate() {
            assert_eq!(stamped.seq, index as u64);
            assert_eq!(stamped.at_ms, 1_000);
        }
        assert_eq!(turn_end_of(&events), (TurnOutcome::Completed, 1, 3));
        let user = find_event(&events, |event| match event {
            SessionEvent::UserMessage {
                text, from_queue, ..
            } => Some((text.clone(), *from_queue)),
            _ => None,
        });
        assert_eq!(user, Some(("hi".to_string(), false)));
        assert_eq!(session.phase(), SessionPhase::Idle);
    }

    #[test]
    fn cancel_between_levels_ends_the_turn_cancelled() {
        let plan = fake_plan_t {
            block_at_level: Some(1),
            ..fake_plan_t::reply("Hello.")
        };
        let backend = Arc::new(fake_backend_t::new(vec![plan]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("hi".to_string());
        let mut events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::LevelDone { level: 0, .. })
        });
        session.cancel();
        events.extend(pump_idle(session.as_mut()));
        let cancel = find_event(&events, |event| match event {
            SessionEvent::Cancel {
                call,
                levels_done,
                scope,
                ..
            } => Some((*call, *levels_done, *scope)),
            _ => None,
        });
        assert_eq!(cancel, Some((Some(0), Some(1), CancelScope::Call)));
        assert_eq!(turn_end_of(&events), (TurnOutcome::Cancelled, 1, 1));
        assert!(!names(&events).contains(&"call_end"));
    }

    #[test]
    fn backend_error_names_the_error_and_fails_the_turn() {
        let plan = fake_plan_t {
            fail_after_passes: Some(1),
            ..fake_plan_t::reply("never")
        };
        let backend = Arc::new(fake_backend_t::new(vec![plan]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("hi".to_string());
        let events = pump_idle(session.as_mut());
        assert_eq!(
            error_named(&events, "backend_failed"),
            Some((ErrorScope::Backend, "backend_failed".to_string()))
        );
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Failed);
    }

    #[test]
    fn invalid_reply_is_retried_with_seed_plus_attempt_then_fails() {
        let bad = "@call no_such_tool\n@end";
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply(bad),
            fake_plan_t::reply(bad),
            fake_plan_t::reply(bad),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("hi".to_string());
        let events = pump_idle(session.as_mut());
        let seeds: Vec<u64> = backend.seen().iter().map(|call| call.seed).collect();
        let first = *seeds.first().expect("a first call");
        assert_eq!(first % 2, 0, "the call seed is even");
        assert_eq!(seeds, vec![first, first + 1, first + 2]);
        assert_eq!(first, 4242 * 2);
        let call_ends = events
            .iter()
            .filter(|stamped| matches!(stamped.event, SessionEvent::CallEnd { valid: false, .. }))
            .count();
        assert_eq!(call_ends, 3);
        assert_eq!(
            error_named(&events, "invalid_reply"),
            Some((ErrorScope::Reply, "invalid_reply".to_string()))
        );
        let message = find_event(&events, |event| match event {
            SessionEvent::Error { name, message, .. } if name == "invalid_reply" => {
                Some(message.clone())
            }
            _ => None,
        })
        .expect("the invalid_reply message");
        assert!(message.starts_with("no valid reply after 3 attempts: "));
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Failed);
    }

    #[test]
    fn ask_mode_asks_for_a_write_then_runs_it_and_continues() {
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply("@call create_file\npath: notes.txt\n@end"),
            fake_plan_t::reply("Created."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("make a file".to_string());
        let mut events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::ApprovalRequested { .. })
        });
        let id = requested_id(&events);
        assert_eq!(session.phase(), SessionPhase::AwaitingApproval);
        let class = find_event(&events, |event| match event {
            SessionEvent::ApprovalRequested { class, tool, .. } if tool == "create_file" => {
                Some(*class)
            }
            _ => None,
        });
        assert_eq!(class, Some(ApprovalClass::Write));
        session.resolve_approval(&id, ApprovalDecision::Once);
        events.extend(pump_idle(session.as_mut()));
        assert!(names(&events).contains(&"tool_result"));
        let second_start = find_event(&events, |event| match event {
            SessionEvent::CallStart {
                call, attempt: 0, ..
            } => Some(*call),
            _ => None,
        });
        assert_eq!(
            second_start,
            Some(0),
            "the second CallStart is call 1, not 0"
        );
        assert_eq!(turn_end_of(&events), (TurnOutcome::Completed, 2, 6));
        assert_eq!(tools.ran().len(), 1);
    }

    #[test]
    fn denied_call_returns_a_tool_error_to_the_next_prefix() {
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply("@call create_file\npath: notes.txt\n@end"),
            fake_plan_t::reply("Understood."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("make a file".to_string());
        let events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::ApprovalRequested { .. })
        });
        let id = requested_id(&events);
        session.resolve_approval(&id, ApprovalDecision::Deny);
        let rest = pump_idle(session.as_mut());
        let denied = find_event(&rest, |event| match event {
            SessionEvent::ToolError { kind, message, .. } => Some((*kind, message.clone())),
            _ => None,
        });
        assert_eq!(
            denied,
            Some((ToolErrorKind::Denied, "denied by user".to_string()))
        );
        assert!(tools.ran().is_empty(), "a denied call never runs");
        assert!(decoded(&backend, 1).contains("denied by user"));
        assert_eq!(turn_end_of(&rest).0, TurnOutcome::Completed);
    }

    #[test]
    fn session_grant_skips_the_second_prompt_for_the_same_path() {
        let reply = "@call create_file\npath: a.txt\n@end\n@call create_file\npath: a.txt\n@end";
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply(reply),
            fake_plan_t::reply("Done."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("twice".to_string());
        let mut events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::ApprovalRequested { .. })
        });
        let id = requested_id(&events);
        session.resolve_approval(&id, ApprovalDecision::Session);
        events.extend(pump_idle(session.as_mut()));
        let requests = events
            .iter()
            .filter(|stamped| matches!(stamped.event, SessionEvent::ApprovalRequested { .. }))
            .count();
        assert_eq!(requests, 1);
        let sources: Vec<ApprovalSource> = events
            .iter()
            .filter_map(|stamped| match &stamped.event {
                SessionEvent::Approval { source, .. } => Some(*source),
                _ => None,
            })
            .collect();
        assert_eq!(
            sources,
            vec![ApprovalSource::User, ApprovalSource::SessionGrant]
        );
        assert_eq!(tools.ran().len(), 2);
    }

    #[test]
    fn edit_mode_allows_a_write_inside_cwd_and_asks_for_shell() {
        let reply = "@call create_file\npath: notes.txt\n@end\n@call run_shell\ncommand: ls\n@end";
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply(reply),
            fake_plan_t::reply("Done."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Edit);
        let mut session = runtime.new_session().expect("a session");
        session.submit("edit and list".to_string());
        let mut events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::ApprovalRequested { .. })
        });
        let mode_allowed = find_event(&events, |event| match event {
            SessionEvent::Approval {
                tool_call_id,
                source,
                ..
            } => Some((tool_call_id.clone(), *source)),
            _ => None,
        });
        assert_eq!(
            mode_allowed,
            Some(("t1.0.0".to_string(), ApprovalSource::Mode))
        );
        let asked = find_event(&events, |event| match event {
            SessionEvent::ApprovalRequested { tool, class, .. } => Some((tool.clone(), *class)),
            _ => None,
        });
        assert_eq!(asked, Some(("run_shell".to_string(), ApprovalClass::Shell)));
        let id = requested_id(&events);
        session.resolve_approval(&id, ApprovalDecision::Once);
        events.extend(pump_idle(session.as_mut()));
        assert_eq!(tools.ran().len(), 2);
    }

    #[test]
    fn question_answer_goes_into_the_next_prefix() {
        let reply = "@ask\nquestion: Which file?\noptions: a.txt | b.txt\n@end";
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply(reply),
            fake_plan_t::reply("Using a.txt."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("which?".to_string());
        let events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::QuestionAsked { .. })
        });
        let qid = question_id(&events);
        assert_eq!(session.phase(), SessionPhase::AwaitingAnswer);
        session.answer_question(&qid, "a.txt".to_string(), Some(0));
        let rest = pump_idle(session.as_mut());
        let answer = find_event(&rest, |event| match event {
            SessionEvent::AskAnswer {
                answer,
                option_index,
                ..
            } => Some((answer.clone(), *option_index)),
            _ => None,
        });
        assert_eq!(answer, Some(("a.txt".to_string(), Some(0))));
        assert!(decoded(&backend, 1).contains("User answer: a.txt"));
        assert_eq!(turn_end_of(&rest).0, TurnOutcome::Completed);
    }

    #[test]
    fn message_queued_during_a_call_enters_the_transcript_before_the_next_call() {
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply("@call list_dir\npath: src\n@end"),
            fake_plan_t::reply("Listed."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("first".to_string());
        session.submit("second".to_string());
        assert_eq!(session.queued(), vec!["second".to_string()]);
        let events = pump_idle(session.as_mut());
        assert!(names(&events).contains(&"message_queued"));
        let queued_at = events
            .iter()
            .position(|stamped| {
                matches!(&stamped.event, SessionEvent::UserMessage { text, from_queue: true, .. } if text == "second")
            })
            .expect("the queued message entered the transcript");
        let second_call = events
            .iter()
            .position(|stamped| matches!(stamped.event, SessionEvent::CallStart { call: 1, .. }))
            .expect("a second call");
        assert!(queued_at < second_call);
        assert!(decoded(&backend, 1).contains("User: second"));
        assert!(session.queued().is_empty());
        assert_eq!(turn_end_of(&events), (TurnOutcome::Completed, 2, 6));
    }

    #[test]
    fn message_queued_during_a_turn_starts_the_next_turn() {
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply("One."),
            fake_plan_t::reply("Two."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("first".to_string());
        session.submit("second".to_string());
        let events = pump_idle(session.as_mut());
        let next = find_event(&events, |event| match event {
            SessionEvent::UserMessage {
                turn,
                text,
                from_queue,
            } if *turn == 2 => Some((text.clone(), *from_queue)),
            _ => None,
        });
        assert_eq!(next, Some(("second".to_string(), true)));
        let ends = events
            .iter()
            .filter(|stamped| {
                matches!(
                    stamped.event,
                    SessionEvent::TurnEnd {
                        outcome: TurnOutcome::Completed,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(ends, 2);
    }

    #[test]
    fn cancel_keeps_the_queued_messages() {
        let plan = fake_plan_t {
            block_at_level: Some(0),
            ..fake_plan_t::reply("never")
        };
        let backend = Arc::new(fake_backend_t::new(vec![plan]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("first".to_string());
        session.submit("second".to_string());
        session.cancel();
        assert_eq!(session.queued(), vec!["second".to_string()]);
        let events = pump_idle(session.as_mut());
        let cancelled_at = events
            .iter()
            .position(|stamped| {
                matches!(
                    stamped.event,
                    SessionEvent::TurnEnd {
                        outcome: TurnOutcome::Cancelled,
                        ..
                    }
                )
            })
            .expect("the first turn was cancelled");
        let next_at = events
            .iter()
            .position(|stamped| {
                matches!(&stamped.event, SessionEvent::UserMessage { turn: 2, text, from_queue: true } if text == "second")
            })
            .expect("the queued message started the next turn");
        assert!(cancelled_at < next_at);
    }

    #[test]
    fn truncated_reply_adds_a_notice_that_names_the_missing_feature() {
        let plan = fake_plan_t {
            tokens: Some(vec![u32::from(b'h'); TestReplyLen as usize]),
            ..fake_plan_t::default()
        };
        let backend = Arc::new(fake_backend_t::new(vec![plan]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("long".to_string());
        let events = pump_idle(session.as_mut());
        let truncated = find_event(&events, |event| match event {
            SessionEvent::CallEnd { truncated, .. } => Some(*truncated),
            _ => None,
        });
        assert_eq!(truncated, Some(true));
        let notice = find_event(&events, |event| match event {
            SessionEvent::Notice { level, text } => Some((*level, text.clone())),
            _ => None,
        })
        .expect("a notice");
        assert_eq!(notice.0, NoticeLevel::Warning);
        assert!(notice.1.contains("continuation of truncated replies (D11)"));
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Completed);
    }

    #[test]
    fn pass_budget_exhaustion_fails_the_turn_with_a_named_error() {
        let bad = "@call no_such_tool\n@end";
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply(bad),
            fake_plan_t::reply(bad),
            fake_plan_t::reply(bad),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let store = Arc::new(memory_store_t::new());
        let mut config = test_config(ApprovalMode::Ask);
        config.pass_budget = 4;
        let runtime = runtime_with(Arc::clone(&backend), Arc::clone(&tools), store, config);
        let mut session = runtime.new_session().expect("a session");
        session.submit("hi".to_string());
        let events = pump_idle(session.as_mut());
        assert_eq!(
            error_named(&events, "pass_budget_exhausted"),
            Some((ErrorScope::Budget, "pass_budget_exhausted".to_string()))
        );
        assert_eq!(turn_end_of(&events), (TurnOutcome::Failed, 1, 6));
        assert_eq!(backend.seen().len(), 2, "the third attempt never starts");
    }

    #[test]
    fn full_mode_is_refused_without_the_flag() {
        let backend = Arc::new(fake_backend_t::new(Vec::new()));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        assert!(matches!(
            session.set_approval_mode(ApprovalMode::Full),
            Err(SessionError::FullRequiresFlag)
        ));
        assert_eq!(session.approval_mode(), ApprovalMode::Ask);
        session
            .set_approval_mode(ApprovalMode::Edit)
            .expect("edit is allowed");
        assert_eq!(session.approval_mode(), ApprovalMode::Edit);
        let events = session.pump();
        assert!(names(&events).contains(&"approval_mode_changed"));
    }

    #[test]
    fn full_mode_is_allowed_when_the_run_started_with_it() {
        let backend = Arc::new(fake_backend_t::new(Vec::new()));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Full);
        let mut session = runtime.new_session().expect("a session");
        session
            .set_approval_mode(ApprovalMode::Full)
            .expect("full was given on the command line");
        assert_eq!(session.approval_mode(), ApprovalMode::Full);
    }

    #[test]
    fn cancel_while_awaiting_approval_drops_the_request() {
        let backend = Arc::new(fake_backend_t::new(vec![fake_plan_t::reply(
            "@call create_file\npath: notes.txt\n@end",
        )]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("make".to_string());
        let mut events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::ApprovalRequested { .. })
        });
        session.cancel();
        events.extend(pump_idle(session.as_mut()));
        let scope = find_event(&events, |event| match event {
            SessionEvent::Cancel { call, scope, .. } => Some((*call, *scope)),
            _ => None,
        });
        assert_eq!(scope, Some((None, CancelScope::Approval)));
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Cancelled);
        assert!(tools.ran().is_empty());
    }

    #[test]
    fn cancel_while_a_tool_runs_stops_the_tool_and_ends_the_turn() {
        let backend = Arc::new(fake_backend_t::new(vec![fake_plan_t::reply(
            "@call list_dir\npath: src\n@end",
        )]));
        let tools = Arc::new(fake_tools_t::new(true));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("list".to_string());
        let mut events = pump_until(session.as_mut(), |event| {
            matches!(event, SessionEvent::Approval { .. })
        });
        assert_eq!(session.phase(), SessionPhase::RunningTool);
        session.cancel();
        events.extend(pump_idle(session.as_mut()));
        let tool_error = find_event(&events, |event| match event {
            SessionEvent::ToolError { kind, .. } => Some(*kind),
            _ => None,
        });
        assert_eq!(tool_error, Some(ToolErrorKind::Cancelled));
        let scope = find_event(&events, |event| match event {
            SessionEvent::Cancel { scope, .. } => Some(*scope),
            _ => None,
        });
        assert_eq!(scope, Some(CancelScope::Tool));
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Cancelled);
    }

    #[test]
    fn every_event_is_stored_and_round_trips_through_json() {
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply("@call list_dir\npath: src\n@end"),
            fake_plan_t::reply("Listed."),
        ]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        let id = session.id().to_string();
        assert!(valid_session_id(&id));
        session.submit("hi".to_string());
        let events = pump_idle(session.as_mut());
        let loaded = store.load(&id).expect("the session was stored");
        assert_eq!(loaded.events, events);
        assert_eq!(loaded.skipped_lines, 0);
        for stamped in &events {
            let line = serde_json::to_string(stamped).expect("an event serialises");
            let back: stamped_event_t = serde_json::from_str(&line).expect("and reads back");
            assert_eq!(&back, stamped);
        }
    }

    #[test]
    fn a_worker_that_panics_reports_worker_lost_and_fails_the_turn() {
        let plan = fake_plan_t {
            panic_call: true,
            ..fake_plan_t::default()
        };
        let backend = Arc::new(fake_backend_t::new(vec![plan]));
        let tools = Arc::new(fake_tools_t::new(false));
        let (runtime, _store) = runtime(&backend, &tools, ApprovalMode::Ask);
        let mut session = runtime.new_session().expect("a session");
        session.submit("hi".to_string());
        let events = pump_idle(session.as_mut());
        assert_eq!(
            error_named(&events, "worker_lost"),
            Some((ErrorScope::Internal, "worker_lost".to_string()))
        );
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Failed);
    }

    #[test]
    fn a_store_failure_is_reported_once_and_the_session_keeps_working() {
        let backend = Arc::new(fake_backend_t::new(vec![fake_plan_t::reply("Fine.")]));
        let tools = Arc::new(fake_tools_t::new(false));
        let store: Arc<dyn session_store_t> = Arc::new(failing_store_t);
        let backend_dyn: Arc<dyn ncm::backend::backend_t> = backend;
        let tools_dyn: Arc<dyn crate::tools::tool_runner_t> = tools;
        let runtime = controller_runtime_t::new(
            backend_dyn,
            tools_dyn,
            store,
            test_config(ApprovalMode::Ask),
        )
        .expect("the test runtime is valid");
        let mut session = runtime.new_session().expect("a session");
        session.submit("hi".to_string());
        let events = pump_idle(session.as_mut());
        let reports = events
            .iter()
            .filter(|stamped| {
                matches!(&stamped.event, SessionEvent::Error { name, .. } if name == "store_write_failed")
            })
            .count();
        assert_eq!(reports, 1);
        assert_eq!(turn_end_of(&events).0, TurnOutcome::Completed);
    }
}
