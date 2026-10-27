/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The runtime that creates, lists and reopens sessions for a backend, a tool runner and a
//! store (PROJECT.md, "UI prototype (P1)", "Event stream and session API").

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use ncm::backend::backend_t;

use crate::api::{SessionError, hint_t, opened_session_t, runtime_info_t, runtime_t, session_t};
use crate::approval::ApprovalMode;
use crate::controller::{controller_session_t, session_deps_t};
use crate::organize::{
    OrganizeOp, memory_organization_store_t, organization_store_t, organization_t,
};
use crate::replay::restore;
use crate::store::{session_store_t, session_summary_t};
use crate::tools::tool_runner_t;

/// Attempts per call: one sample and two retries (D46).
pub const MaxAttempts: u32 = 3;

/// Level passes per turn (D46).
pub const PassBudget: u32 = 64;

/// The session seed when the caller does not choose one (D42).
pub const DefaultSeed: u64 = 4242;

const FnvOffset: u64 = 0xcbf2_9ce4_8422_2325;
const FnvPrime: u64 = 0x0000_0100_0000_01b3;

/// Counts the sessions this process has created, so two ids of one process never share input.
static IdCounter: AtomicU64 = AtomicU64::new(0);

/// The settings of one run. The UI and the headless modes fill it in.
#[derive(Clone, Debug)]
pub struct runtime_config_t {
    pub cwd: PathBuf,
    /// The mode the run was started with. `Full` is allowed only when this is `Full`.
    pub approval: ApprovalMode,
    pub session_seed: u64,
    pub max_attempts: u32,
    pub pass_budget: u32,
    pub version: String,
    /// Copied into `runtime_info_t.notice`.
    pub notice: Option<String>,
    /// Written into `session_start.label`.
    pub label: Option<String>,
    pub state_note: Option<String>,
    pub hints: Vec<hint_t>,
    /// Milliseconds since the Unix epoch. `system_clock` is the usual choice.
    pub clock: fn() -> u64,
}

/// The current time in milliseconds since the Unix epoch, or 0 when the clock is before it.
pub fn system_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The last component of the working directory, or the whole path when it has none.
pub fn project_name(cwd: &Path) -> String {
    match cwd.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => cwd.to_string_lossy().into_owned(),
    }
}

/// A new session id: `s_` and 16 lowercase hex digits. The id is FNV-1a (64 bit) over the clock,
/// the nanoseconds and the process-wide counter. The counter is the last input, so two ids made
/// by one process differ whenever their clock and nanoseconds agree.
fn new_session_id(clock_ms: u64) -> String {
    let counter = IdCounter.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    let mut hash = FnvOffset;
    let inputs = clock_ms
        .to_le_bytes()
        .into_iter()
        .chain(u64::from(nanos).to_le_bytes())
        .chain(counter.to_le_bytes());
    for byte in inputs {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FnvPrime);
    }
    format!("s_{hash:016x}")
}

/// The runtime for one backend, one tool runner and one store.
pub struct controller_runtime_t {
    deps: session_deps_t,
    info: runtime_info_t,
    organization: Arc<dyn organization_store_t>,
    /// Held across a load, a change and a save, so two changes never interleave.
    organize_lock: Mutex<()>,
}

impl controller_runtime_t {
    /// Checks the backend's capabilities, then builds the runtime. An inconsistent set is
    /// refused here, so no session starts on a model whose limits do not hold.
    pub fn new(
        backend: Arc<dyn backend_t>,
        tools: Arc<dyn tool_runner_t>,
        store: Arc<dyn session_store_t>,
        config: runtime_config_t,
    ) -> Result<Self, SessionError> {
        backend.capabilities().validate()?;
        let cwd = config.cwd.to_string_lossy().into_owned();
        let info = runtime_info_t {
            version: config.version.clone(),
            identity: backend.identity().clone(),
            capabilities: backend.capabilities().clone(),
            cwd,
            project: project_name(&config.cwd),
            approval_mode: config.approval,
            notice: config.notice.clone(),
            state_note: config.state_note.clone(),
            hints: config.hints.clone(),
        };
        let deps = session_deps_t {
            backend,
            runner: tools,
            store,
            config,
        };
        Ok(controller_runtime_t {
            deps,
            info,
            organization: Arc::new(memory_organization_store_t::new()),
            organize_lock: Mutex::new(()),
        })
    }

    /// Keeps the folders in `store` instead of in memory (D50).
    pub fn with_organization_store(mut self, store: Arc<dyn organization_store_t>) -> Self {
        self.organization = store;
        self
    }
}

impl runtime_t for controller_runtime_t {
    fn info(&self) -> &runtime_info_t {
        &self.info
    }

    fn list_sessions(&self) -> Result<Vec<session_summary_t>, SessionError> {
        Ok(self.deps.store.list()?)
    }

    fn new_session(&self) -> Result<Box<dyn session_t>, SessionError> {
        let id = new_session_id((self.deps.config.clock)());
        Ok(Box::new(controller_session_t::start(self.deps.clone(), id)))
    }

    /// Reads a session back. The events are returned first, then any event the reopening
    /// itself wrote, such as the notice and the closing `turn_end` of a turn that the last run
    /// did not finish. Those events are in the store too.
    fn open_session(&self, id: &str) -> Result<opened_session_t, SessionError> {
        let loaded = self.deps.store.load(id)?;
        let restored = restore(&loaded.events).ok_or_else(|| SessionError::Internal {
            message: format!("session {id} has no session_start event"),
        })?;
        let mut session = controller_session_t::resume(self.deps.clone(), id.to_string(), restored);
        let mut events = loaded.events;
        events.extend(session.take_events());
        Ok(opened_session_t {
            session: Box::new(session),
            events,
            skipped_lines: loaded.skipped_lines,
        })
    }

    fn organization(&self) -> Result<organization_t, SessionError> {
        Ok(self.organization.load()?)
    }

    fn organize(&self, op: OrganizeOp) -> Result<organization_t, SessionError> {
        let _held = self
            .organize_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut organization = self.organization.load()?;
        organization.apply(&op)?;
        self.organization.save(&organization)?;
        Ok(organization)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ncm::backend::{BackendError, backend_t};

    use crate::api::{SessionError, runtime_t};
    use crate::approval::ApprovalMode;
    use crate::controller::test_support::*;
    use crate::event::SessionEvent;
    use crate::organize::{
        OrganizeError, OrganizeOp, memory_organization_store_t, organization_store_t,
        organization_t,
    };
    use crate::store::{memory_store_t, session_store_t, valid_session_id};

    use super::{controller_runtime_t, system_clock};

    fn started_call(events: &[crate::event::stamped_event_t], wanted: u32) -> Option<u32> {
        find_event(events, |event| match event {
            SessionEvent::CallStart {
                call, attempt: 0, ..
            } if *call == wanted => Some(*call),
            _ => None,
        })
    }

    #[test]
    fn open_session_continues_the_turn_call_and_sequence_numbers() {
        let backend = Arc::new(fake_backend_t::new(vec![
            fake_plan_t::reply("Reply one."),
            fake_plan_t::reply("Reply two."),
        ]));
        let store = Arc::new(memory_store_t::new());
        let runtime = runtime_with(
            Arc::clone(&backend),
            Arc::new(fake_tools_t::new(false)),
            Arc::clone(&store),
            test_config(ApprovalMode::Ask),
        );
        let mut first = runtime.new_session().expect("a session");
        let id = first.id().to_string();
        first.submit("one".to_string());
        let first_events = pump_idle(first.as_mut());
        let last_seq = first_events.last().expect("events").seq;

        let listed = runtime.list_sessions().expect("the store lists sessions");
        assert!(listed.iter().any(|summary| summary.id == id));

        let opened = runtime.open_session(&id).expect("the session opens");
        assert_eq!(opened.events, first_events);
        assert_eq!(opened.skipped_lines, 0);
        let mut reopened = opened.session;
        reopened.submit("two".to_string());
        let events = pump_idle(reopened.as_mut());

        assert_eq!(
            events.first().map(|stamped| stamped.seq),
            Some(last_seq + 1)
        );
        let turn = find_event(&events, |event| match event {
            SessionEvent::TurnStart { turn } => Some(*turn),
            _ => None,
        });
        assert_eq!(turn, Some(2));
        assert!(
            started_call(&events, 1).is_some(),
            "the next call is call 1"
        );
        let seen = backend.seen();
        let second = seen
            .get(1)
            .expect("the reopened session called the backend");
        assert_eq!(second.seed, 4242 * 2 + 1024);
        assert!(bytes_to_text(&second.prefix).contains("Reply one."));
    }

    #[test]
    fn open_session_closes_a_turn_the_last_run_did_not_finish() {
        let plan = fake_plan_t {
            block_at_level: Some(0),
            ..fake_plan_t::reply("never")
        };
        let backend = Arc::new(fake_backend_t::new(vec![plan]));
        let store = Arc::new(memory_store_t::new());
        let runtime = runtime_with(
            backend,
            Arc::new(fake_tools_t::new(false)),
            Arc::clone(&store),
            test_config(ApprovalMode::Ask),
        );
        let id = {
            let mut session = runtime.new_session().expect("a session");
            session.submit("interrupted".to_string());
            session.id().to_string()
        };
        let opened = runtime.open_session(&id).expect("the session opens");
        let tail = names(&opened.events);
        assert!(
            tail.ends_with(&["notice", "turn_end"]),
            "the recovery is appended, saw {tail:?}"
        );
        let closing = opened.events.last().expect("events");
        assert!(matches!(
            closing.event,
            SessionEvent::TurnEnd {
                outcome: crate::event::TurnOutcome::Failed,
                calls: 0,
                passes: 0,
                ..
            }
        ));
        let stored = store.load(&id).expect("the session is stored");
        assert_eq!(
            stored.events, opened.events,
            "the recovery is in the log too"
        );

        let mut session = opened.session;
        session.submit("again".to_string());
        let events = pump_idle(session.as_mut());
        assert!(names(&events).contains(&"turn_start"));
    }

    #[test]
    fn a_full_session_reopened_without_the_flag_drops_to_the_run_mode() {
        let backend = Arc::new(fake_backend_t::new(Vec::new()));
        let store = Arc::new(memory_store_t::new());
        let full = runtime_with(
            Arc::clone(&backend),
            Arc::new(fake_tools_t::new(false)),
            Arc::clone(&store),
            test_config(ApprovalMode::Full),
        );
        let session = full.new_session().expect("a session");
        let id = session.id().to_string();
        drop(session);

        let plain = runtime_with(
            backend,
            Arc::new(fake_tools_t::new(false)),
            store,
            test_config(ApprovalMode::Ask),
        );
        let opened = plain.open_session(&id).expect("the session opens");
        assert_eq!(opened.session.approval_mode(), ApprovalMode::Ask);
        let seen = names(&opened.events);
        assert!(seen.contains(&"approval_mode_changed"));
        assert!(seen.contains(&"notice"));
    }

    #[test]
    fn opening_an_unknown_session_is_a_store_error() {
        let backend = Arc::new(fake_backend_t::new(Vec::new()));
        let runtime = runtime_with(
            backend,
            Arc::new(fake_tools_t::new(false)),
            Arc::new(memory_store_t::new()),
            test_config(ApprovalMode::Ask),
        );
        assert!(matches!(
            runtime.open_session("s_0000000000000000"),
            Err(SessionError::Store(_))
        ));
    }

    #[test]
    fn new_refuses_inconsistent_capabilities_with_a_backend_error() {
        let mut caps = test_caps();
        caps.level_lens = vec![32, 128];
        let backend: Arc<dyn backend_t> =
            Arc::new(fake_backend_t::with_capabilities(Vec::new(), caps));
        let result = controller_runtime_t::new(
            backend,
            Arc::new(fake_tools_t::new(false)),
            Arc::new(memory_store_t::new()),
            test_config(ApprovalMode::Ask),
        );
        assert!(matches!(
            result,
            Err(SessionError::Backend(
                BackendError::InvalidCapabilities { .. }
            ))
        ));
    }

    #[test]
    fn info_names_the_project_from_the_working_directory() {
        let backend = Arc::new(fake_backend_t::new(Vec::new()));
        let mut config = test_config(ApprovalMode::Edit);
        config.notice = Some("a notice".to_string());
        let runtime = runtime_with(
            backend,
            Arc::new(fake_tools_t::new(false)),
            Arc::new(memory_store_t::new()),
            config,
        );
        let info = runtime.info();
        assert_eq!(info.project, "project");
        assert_eq!(info.cwd, "/srv/ncagent-test/project");
        assert_eq!(info.approval_mode, ApprovalMode::Edit);
        assert_eq!(info.notice.as_deref(), Some("a notice"));
        assert_eq!(info.version, "test");
        assert_eq!(info.capabilities.reply_len, TestReplyLen);
    }

    #[test]
    fn new_session_ids_are_valid_and_distinct() {
        let backend = Arc::new(fake_backend_t::new(Vec::new()));
        let runtime = runtime_with(
            backend,
            Arc::new(fake_tools_t::new(false)),
            Arc::new(memory_store_t::new()),
            test_config(ApprovalMode::Ask),
        );
        let mut seen = HashSet::new();
        for _ in 0..50 {
            let session = runtime.new_session().expect("a session");
            let id = session.id().to_string();
            assert!(valid_session_id(&id));
            let hex = id.strip_prefix("s_").unwrap_or("");
            assert_eq!(hex.len(), 16);
            assert!(
                hex.bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
            assert!(seen.insert(id), "two sessions share an id");
        }
    }

    #[test]
    fn system_clock_reads_milliseconds_since_the_epoch() {
        // 2020-09-13 in milliseconds. Any real clock is later than that.
        assert!(system_clock() > 1_600_000_000_000);
    }

    /// A runtime whose folders are kept in `folders`.
    fn runtime_keeping_folders_in(folders: Arc<dyn organization_store_t>) -> controller_runtime_t {
        runtime_with(
            Arc::new(fake_backend_t::new(Vec::new())),
            Arc::new(fake_tools_t::new(false)),
            Arc::new(memory_store_t::new()),
            test_config(ApprovalMode::Ask),
        )
        .with_organization_store(folders)
    }

    /// A store whose load always fails. It counts the saves, which must stay at zero.
    struct failing_store_t {
        saves: AtomicUsize,
    }

    impl organization_store_t for failing_store_t {
        fn load(&self) -> Result<organization_t, OrganizeError> {
            Err(OrganizeError::Io {
                path: "folders.json".to_string(),
                message: "simulated read failure".to_string(),
            })
        }

        fn save(&self, _organization: &organization_t) -> Result<(), OrganizeError> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn a_fresh_runtime_has_no_folders() {
        let runtime = runtime_with(
            Arc::new(fake_backend_t::new(Vec::new())),
            Arc::new(fake_tools_t::new(false)),
            Arc::new(memory_store_t::new()),
            test_config(ApprovalMode::Ask),
        );
        assert_eq!(
            runtime.organization().expect("the folders load"),
            organization_t::new()
        );
    }

    #[test]
    fn organize_applies_a_change_and_another_runtime_on_the_same_store_sees_it() {
        let folders = Arc::new(memory_organization_store_t::new());
        let first = runtime_keeping_folders_in(folders.clone());
        let after = first
            .organize(OrganizeOp::CreateFolder {
                name: "Research".to_string(),
                parent: None,
            })
            .expect("the folder is created");
        assert_eq!(after.children(None).len(), 1);
        assert_eq!(after.children(None)[0].name, "Research");

        let second = runtime_keeping_folders_in(folders);
        assert_eq!(second.organization().expect("the folders load"), after);
    }

    #[test]
    fn a_refused_change_returns_its_error_and_keeps_the_folders() {
        let runtime = runtime_keeping_folders_in(Arc::new(memory_organization_store_t::new()));
        runtime
            .organize(OrganizeOp::CreateFolder {
                name: "Outer".to_string(),
                parent: None,
            })
            .expect("the outer folder is created");
        runtime
            .organize(OrganizeOp::CreateFolder {
                name: "Inner".to_string(),
                parent: Some("f1".to_string()),
            })
            .expect("the inner folder is created");
        let before = runtime.organization().expect("the folders load");

        let refused = runtime.organize(OrganizeOp::MoveFolder {
            id: "f1".to_string(),
            parent: Some("f2".to_string()),
        });
        assert!(matches!(
            refused,
            Err(SessionError::Organize(OrganizeError::WouldNest { .. }))
        ));
        assert_eq!(runtime.organization().expect("the folders load"), before);
    }

    #[test]
    fn folders_that_do_not_load_are_never_saved_over() {
        let store = Arc::new(failing_store_t {
            saves: AtomicUsize::new(0),
        });
        let runtime = runtime_keeping_folders_in(store.clone());

        let result = runtime.organize(OrganizeOp::CreateFolder {
            name: "New".to_string(),
            parent: None,
        });
        assert!(matches!(
            result,
            Err(SessionError::Organize(OrganizeError::Io { .. }))
        ));
        assert!(runtime.organization().is_err());
        assert_eq!(store.saves.load(Ordering::SeqCst), 0);
    }
}
