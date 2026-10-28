/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Where a session's events are kept (D24).
//!
//! The harness defines the trait. The demo's file-backed store lives in `crates/demo` (D40).
//! A store treats what it reads as untrusted: a malformed line is skipped and counted, never a
//! panic.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::event::{SessionEvent, stamped_event_t};

/// Longest session title, in characters.
pub const TitleLimit: usize = 60;

/// Longest session id, in bytes.
const SessionIdLimit: usize = 64;

/// One line of the session list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct session_summary_t {
    pub id: String,
    /// The first user message, cut short. Empty until there is one.
    pub title: String,
    /// The last component of the session's working directory.
    pub project: String,
    pub model_id: String,
    pub simulated: bool,
    /// Time of the last event, in milliseconds since the Unix epoch.
    pub updated_ms: u64,
    pub turns: u32,
    pub label: Option<String>,
}

/// A session read back from a store.
#[derive(Clone, Debug, PartialEq)]
pub struct loaded_session_t {
    pub events: Vec<stamped_event_t>,
    /// Lines that did not parse and were skipped.
    pub skipped_lines: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    Io {
        path: String,
        message: String,
    },
    NotFound {
        id: String,
    },
    /// The id is not a plain session id, so it could name a path outside the store.
    InvalidId {
        id: String,
    },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io { path, message } => write!(f, "store i/o error at {path}: {message}"),
            StoreError::NotFound { id } => write!(f, "no stored session {id}"),
            StoreError::InvalidId { id } => write!(f, "invalid session id {id:?}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The summary of a session, from its events. `None` when there is no `session_start`.
///
/// Every store uses this function, so a session looks the same in the sidebar whichever store
/// holds it.
pub fn summarize_events(id: &str, events: &[stamped_event_t]) -> Option<session_summary_t> {
    let mut summary = None;
    let mut title = String::new();
    let mut turns = 0u32;
    let mut updated_ms = 0u64;
    for stamped in events {
        updated_ms = updated_ms.max(stamped.at_ms);
        match &stamped.event {
            SessionEvent::SessionStart {
                simulated,
                label,
                model_id,
                project,
                ..
            } if summary.is_none() => {
                summary = Some((project.clone(), model_id.clone(), *simulated, label.clone()));
            }
            SessionEvent::UserMessage { text, .. } if title.is_empty() => {
                let first_line = text.lines().next().unwrap_or("").trim();
                title = first_line.chars().take(TitleLimit).collect();
            }
            SessionEvent::TurnStart { .. } => turns = turns.saturating_add(1),
            _ => {}
        }
    }
    let (project, model_id, simulated, label) = summary?;
    Some(session_summary_t {
        id: id.to_string(),
        title,
        project,
        model_id,
        simulated,
        updated_ms,
        turns,
        label,
    })
}

pub trait session_store_t: Send + Sync {
    /// Every stored session, newest first.
    fn list(&self) -> Result<Vec<session_summary_t>, StoreError>;

    fn load(&self, id: &str) -> Result<loaded_session_t, StoreError>;

    fn append(&self, id: &str, event: &stamped_event_t) -> Result<(), StoreError>;

    /// Removes a session's stored log. `NotFound` when there is none. Only that session's log is
    /// removed (D59).
    fn delete(&self, id: &str) -> Result<(), StoreError>;
}

/// True for an id of 1 to 64 bytes, each an ASCII letter, a digit, `_` or `-`. A store that
/// builds a path from an id must refuse anything else, so an id cannot name a place outside it.
pub fn valid_session_id(id: &str) -> bool {
    (1..=SessionIdLimit).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn check_id(id: &str) -> Result<(), StoreError> {
    if valid_session_id(id) {
        Ok(())
    } else {
        Err(StoreError::InvalidId { id: id.to_string() })
    }
}

/// A session store that keeps events in memory until the process ends.
#[derive(Debug, Default)]
pub struct memory_store_t {
    sessions: Mutex<HashMap<String, Vec<stamped_event_t>>>,
}

impl memory_store_t {
    pub fn new() -> Self {
        Self::default()
    }

    /// The map, even when a panic poisoned the lock. Each entry is a whole event list, so the
    /// map is never left half-written and reading it after a panic is safe.
    fn locked(&self) -> MutexGuard<'_, HashMap<String, Vec<stamped_event_t>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl session_store_t for memory_store_t {
    fn list(&self) -> Result<Vec<session_summary_t>, StoreError> {
        let sessions = self.locked();
        let mut summaries: Vec<session_summary_t> = sessions
            .iter()
            .filter_map(|(id, events)| summarize_events(id, events))
            .collect();
        summaries.sort_by(|a, b| {
            b.updated_ms
                .cmp(&a.updated_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(summaries)
    }

    fn load(&self, id: &str) -> Result<loaded_session_t, StoreError> {
        check_id(id)?;
        match self.locked().get(id) {
            Some(events) => Ok(loaded_session_t {
                events: events.clone(),
                skipped_lines: 0,
            }),
            None => Err(StoreError::NotFound { id: id.to_string() }),
        }
    }

    fn append(&self, id: &str, event: &stamped_event_t) -> Result<(), StoreError> {
        check_id(id)?;
        self.locked()
            .entry(id.to_string())
            .or_default()
            .push(event.clone());
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), StoreError> {
        check_id(id)?;
        match self.locked().remove(id) {
            Some(_) => Ok(()),
            None => Err(StoreError::NotFound { id: id.to_string() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::ApprovalMode;
    use ncm::backend::BackendKind;

    fn ok<T, E: fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("expected Ok, found {err:?}"),
        }
    }

    fn start(id: &str, at_ms: u64) -> stamped_event_t {
        stamped_event_t {
            seq: 0,
            at_ms,
            event: SessionEvent::SessionStart {
                session_id: id.to_string(),
                simulated: true,
                label: Some("DEMO: simulated model, not NCM output".to_string()),
                model_id: "demo".to_string(),
                backend_kind: BackendKind::Demo,
                contract_version: 1,
                approval_mode: ApprovalMode::Ask,
                cwd: "/work/dir".to_string(),
                project: "dir".to_string(),
                prefix_window: 4096,
                reply_len: 1024,
                level_count: 5,
                level_lens: vec![8, 32, 128, 512, 1024],
                session_seed: 1,
            },
        }
    }

    fn message(at_ms: u64, text: &str) -> stamped_event_t {
        stamped_event_t {
            seq: 1,
            at_ms,
            event: SessionEvent::UserMessage {
                turn: 1,
                text: text.to_string(),
                from_queue: false,
            },
        }
    }

    #[test]
    fn session_ids_must_be_short_plain_ascii() {
        assert!(valid_session_id("abc-DEF_09"));
        let longest = "a".repeat(64);
        assert!(valid_session_id(&longest));
        let too_long = "a".repeat(65);
        for bad in [
            "",
            "../etc",
            "a/b",
            "a b",
            "é",
            "a.b",
            "a\0",
            too_long.as_str(),
        ] {
            assert!(!valid_session_id(bad), "{bad:?} must be refused");
        }
    }

    #[test]
    fn an_appended_session_loads_back_in_order() {
        let store = memory_store_t::new();
        ok(store.append("s1", &start("s1", 1)));
        ok(store.append("s1", &message(2, "hello")));
        let loaded = ok(store.load("s1"));
        assert_eq!(loaded.events.len(), 2);
        assert_eq!(loaded.skipped_lines, 0);
        assert_eq!(loaded.events[0], start("s1", 1));
        assert_eq!(loaded.events[1], message(2, "hello"));
    }

    #[test]
    fn an_unknown_session_is_not_found() {
        let store = memory_store_t::new();
        assert!(matches!(
            store.load("missing"),
            Err(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn an_invalid_id_is_refused_by_append_and_load() {
        let store = memory_store_t::new();
        assert!(matches!(
            store.append("../escape", &start("x", 1)),
            Err(StoreError::InvalidId { .. })
        ));
        assert!(matches!(
            store.load("a/b"),
            Err(StoreError::InvalidId { .. })
        ));
        assert!(ok(store.list()).is_empty());
    }

    #[test]
    fn list_is_newest_first_with_ties_broken_by_id() {
        let store = memory_store_t::new();
        ok(store.append("a", &start("a", 10)));
        ok(store.append("c", &start("c", 20)));
        ok(store.append("b", &start("b", 20)));
        let ids: Vec<String> = ok(store.list())
            .into_iter()
            .map(|summary| summary.id)
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn list_uses_the_last_event_time_and_skips_sessions_without_a_start() {
        let store = memory_store_t::new();
        ok(store.append("real", &start("real", 1)));
        ok(store.append("real", &message(50, "later")));
        ok(store.append("orphan", &message(60, "no start event")));
        let summaries = ok(store.list());
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "real");
        assert_eq!(summaries[0].updated_ms, 50);
        assert_eq!(summaries[0].title, "later");
    }

    #[test]
    fn a_deleted_session_is_not_found_and_leaves_the_list() {
        let store = memory_store_t::new();
        ok(store.append("keep", &start("keep", 1)));
        ok(store.append("gone", &start("gone", 2)));
        ok(store.delete("gone"));
        assert!(matches!(
            store.load("gone"),
            Err(StoreError::NotFound { .. })
        ));
        let ids: Vec<String> = ok(store.list()).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, vec!["keep"]);
        assert!(matches!(
            store.delete("gone"),
            Err(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn delete_refuses_an_invalid_id() {
        let store = memory_store_t::new();
        ok(store.append("keep", &start("keep", 1)));
        assert!(matches!(
            store.delete("../keep"),
            Err(StoreError::InvalidId { .. })
        ));
        assert_eq!(ok(store.load("keep")).events.len(), 1);
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let store = memory_store_t::new();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = store.sessions.lock();
            panic!("poison the lock");
        }));
        ok(store.append("s1", &start("s1", 1)));
        assert_eq!(ok(store.list()).len(), 1);
        assert_eq!(ok(store.load("s1")).events.len(), 1);
    }
}
