/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Rebuilds a session's state from its stored events (D24, PROJECT.md, "Sessions, events and
//! persistence").
//!
//! A resumed session replays the events that the UI folds, and the controller needs the same
//! facts the live path kept: the turn and call counters, the transcript the prompt is packed
//! from, the session seed, the approval mode and the next sequence number. `restore` reads the
//! log once and returns them. Every entry it builds matches an entry the live path pushes, so a
//! resumed prompt equals the prompt the session would have packed.

use crate::approval::ApprovalMode;
use crate::event::{SessionEvent, stamped_event_t};
use crate::pack::TranscriptEntry;

/// The state a session recovers from its events.
pub struct restored_t {
    /// The number of the last turn that started. The next turn is `turn + 1`.
    pub turn: u32,
    /// The number the next call gets. The first call of a session is call 0.
    pub call_index: u32,
    /// The transcript the prompt is packed from, oldest first.
    pub entries: Vec<TranscriptEntry>,
    pub session_seed: u64,
    /// The last `approval_mode_changed`, or the mode in `session_start` when there is none.
    pub approval_mode: ApprovalMode,
    /// The sequence number of the next event.
    pub next_seq: u64,
    /// True when the log holds a `turn_start` with no `turn_end` after it.
    pub mid_turn: bool,
}

/// Reads a session's events. Returns `None` when there is no `session_start`, because such a log
/// does not describe a session.
pub fn restore(events: &[stamped_event_t]) -> Option<restored_t> {
    let mut session_seed: Option<u64> = None;
    let mut approval_mode: Option<ApprovalMode> = None;
    let mut turn = 0u32;
    let mut call_index = 0u32;
    let mut entries = Vec::new();
    let mut next_seq = 0u64;
    let mut mid_turn = false;
    for stamped in events {
        next_seq = next_seq.max(stamped.seq.saturating_add(1));
        match &stamped.event {
            SessionEvent::SessionStart {
                session_seed: seed,
                approval_mode: mode,
                ..
            } => {
                if session_seed.is_none() {
                    session_seed = Some(*seed);
                    approval_mode = Some(*mode);
                }
            }
            SessionEvent::ApprovalModeChanged { mode } => approval_mode = Some(*mode),
            SessionEvent::UserMessage {
                turn: message_turn,
                text,
                ..
            } => {
                turn = turn.max(*message_turn);
                entries.push(TranscriptEntry::User { text: text.clone() });
            }
            SessionEvent::TurnStart { turn: started } => {
                turn = turn.max(*started);
                mid_turn = true;
            }
            SessionEvent::TurnEnd { .. } => mid_turn = false,
            SessionEvent::CallStart { call, .. } => {
                call_index = call_index.max(call.saturating_add(1));
            }
            SessionEvent::CallEnd {
                valid: true, text, ..
            } => entries.push(TranscriptEntry::Assistant { text: text.clone() }),
            SessionEvent::ToolResult { tool, output, .. } => {
                entries.push(TranscriptEntry::ToolResult {
                    tool: tool.clone(),
                    text: output.clone(),
                })
            }
            SessionEvent::ToolError { tool, message, .. } => {
                entries.push(TranscriptEntry::ToolError {
                    tool: tool.clone(),
                    message: message.clone(),
                })
            }
            SessionEvent::AskAnswer { answer, .. } => entries.push(TranscriptEntry::UserAnswer {
                text: answer.clone(),
            }),
            _ => {}
        }
    }
    Some(restored_t {
        turn,
        call_index,
        entries,
        session_seed: session_seed?,
        approval_mode: approval_mode?,
        next_seq,
        mid_turn,
    })
}

#[cfg(test)]
mod tests {
    use super::restore;
    use crate::approval::ApprovalMode;
    use crate::event::{NoticeLevel, ReplyItem, SessionEvent, TurnOutcome, stamped_event_t};
    use crate::pack::TranscriptEntry;
    use crate::tools::ToolErrorKind;
    use ncm::backend::{BackendKind, sampler_t};

    fn stamped(seq: u64, event: SessionEvent) -> stamped_event_t {
        stamped_event_t {
            seq,
            at_ms: 0,
            event,
        }
    }

    fn start(seq: u64, seed: u64, mode: ApprovalMode) -> stamped_event_t {
        stamped(
            seq,
            SessionEvent::SessionStart {
                session_id: "s_0000000000000001".to_string(),
                simulated: true,
                label: None,
                model_id: "test-model".to_string(),
                backend_kind: BackendKind::Demo,
                contract_version: 1,
                approval_mode: mode,
                cwd: "/srv/project".to_string(),
                project: "project".to_string(),
                prefix_window: 64,
                reply_len: 16,
                level_count: 2,
                level_lens: vec![8, 16],
                session_seed: seed,
            },
        )
    }

    fn call_start(seq: u64, turn: u32, call: u32) -> stamped_event_t {
        stamped(
            seq,
            SessionEvent::CallStart {
                turn,
                call,
                attempt: 0,
                seed: 0,
                prompt_hash: "0000000000000000".to_string(),
                packed_tokens: 1,
                window: 64,
                dropped_turns: 0,
                sampler: sampler_t::greedy(),
            },
        )
    }

    #[test]
    fn restore_needs_a_session_start() {
        assert!(restore(&[]).is_none());
        let orphan = stamped(0, SessionEvent::TurnStart { turn: 1 });
        assert!(restore(&[orphan]).is_none());
    }

    #[test]
    fn restore_maps_entries_and_counters_and_the_last_mode() {
        let events = vec![
            start(0, 4242, ApprovalMode::Ask),
            stamped(
                1,
                SessionEvent::ApprovalModeChanged {
                    mode: ApprovalMode::Edit,
                },
            ),
            stamped(
                2,
                SessionEvent::UserMessage {
                    turn: 1,
                    text: "hi".to_string(),
                    from_queue: false,
                },
            ),
            stamped(3, SessionEvent::TurnStart { turn: 1 }),
            call_start(4, 1, 0),
            stamped(
                5,
                SessionEvent::CallEnd {
                    turn: 1,
                    call: 0,
                    attempt: 0,
                    text: "Hello".to_string(),
                    truncated: false,
                    valid: true,
                    issues: Vec::new(),
                    items: vec![ReplyItem::Text {
                        text: "Hello".to_string(),
                    }],
                    passes: 2,
                },
            ),
            stamped(
                6,
                SessionEvent::CallEnd {
                    turn: 1,
                    call: 0,
                    attempt: 1,
                    text: "bad".to_string(),
                    truncated: false,
                    valid: false,
                    issues: vec!["unterminated".to_string()],
                    items: Vec::new(),
                    passes: 2,
                },
            ),
            stamped(
                7,
                SessionEvent::ToolResult {
                    turn: 1,
                    tool_call_id: "t1.0.0".to_string(),
                    tool: "list_dir".to_string(),
                    ok: true,
                    output: "a b".to_string(),
                    truncated: false,
                    simulated: true,
                },
            ),
            stamped(
                8,
                SessionEvent::ToolError {
                    turn: 1,
                    tool_call_id: "t1.0.1".to_string(),
                    tool: "read_file".to_string(),
                    kind: ToolErrorKind::Failed,
                    message: "no such file".to_string(),
                },
            ),
            stamped(
                9,
                SessionEvent::AskAnswer {
                    turn: 1,
                    question_id: "q1.0.2".to_string(),
                    answer: "yes".to_string(),
                    option_index: Some(0),
                },
            ),
            stamped(
                10,
                SessionEvent::Notice {
                    level: NoticeLevel::Warning,
                    text: "ignored".to_string(),
                },
            ),
            stamped(
                11,
                SessionEvent::TurnEnd {
                    turn: 1,
                    outcome: TurnOutcome::Completed,
                    calls: 1,
                    passes: 2,
                },
            ),
            stamped(
                12,
                SessionEvent::UserMessage {
                    turn: 2,
                    text: "again".to_string(),
                    from_queue: true,
                },
            ),
            stamped(13, SessionEvent::TurnStart { turn: 2 }),
            call_start(14, 2, 1),
        ];
        let restored = restore(&events).expect("the log has a session_start");
        assert_eq!(restored.turn, 2);
        assert_eq!(restored.call_index, 2);
        assert_eq!(restored.session_seed, 4242);
        assert_eq!(restored.approval_mode, ApprovalMode::Edit);
        assert_eq!(restored.next_seq, 15);
        assert!(restored.mid_turn);
        assert_eq!(
            restored.entries,
            vec![
                TranscriptEntry::User {
                    text: "hi".to_string()
                },
                TranscriptEntry::Assistant {
                    text: "Hello".to_string()
                },
                TranscriptEntry::ToolResult {
                    tool: "list_dir".to_string(),
                    text: "a b".to_string()
                },
                TranscriptEntry::ToolError {
                    tool: "read_file".to_string(),
                    message: "no such file".to_string()
                },
                TranscriptEntry::UserAnswer {
                    text: "yes".to_string()
                },
                TranscriptEntry::User {
                    text: "again".to_string()
                },
            ]
        );
    }

    #[test]
    fn restore_keeps_the_mode_of_session_start_when_nothing_changed_it() {
        let events = vec![start(0, 7, ApprovalMode::Full)];
        let restored = restore(&events).expect("the log has a session_start");
        assert_eq!(restored.approval_mode, ApprovalMode::Full);
        assert_eq!(restored.call_index, 0);
        assert_eq!(restored.turn, 0);
        assert!(!restored.mid_turn);
    }
}
