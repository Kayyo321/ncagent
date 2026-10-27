/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! What the UI may call. The UI depends on these traits and on the event types, and on nothing
//! else in `harness`, so a test can drive it with a scripted fake and no backend (PROJECT.md,
//! "UI prototype (P1)").

use std::fmt;

use ncm::backend::{BackendError, capabilities_t, model_identity_t};

use crate::approval::{ApprovalDecision, ApprovalMode};
use crate::event::stamped_event_t;
use crate::store::{StoreError, session_summary_t};

/// Where a session is in its turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionPhase {
    Idle,
    /// A call to the backend is running.
    Generating,
    AwaitingApproval,
    RunningTool,
    AwaitingAnswer,
    /// A cancel was requested and the call has not stopped yet.
    Cancelling,
}

impl SessionPhase {
    /// True for every phase except `Idle`.
    pub fn busy(self) -> bool {
        !matches!(self, SessionPhase::Idle)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    Store(StoreError),
    Backend(BackendError),
    /// `full` can be selected only on the command line.
    FullRequiresFlag,
    NotFound {
        id: String,
    },
    Internal {
        message: String,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::Store(err) => write!(f, "{err}"),
            SessionError::Backend(err) => write!(f, "{err}"),
            SessionError::FullRequiresFlag => {
                write!(
                    f,
                    "approval mode `full` must be selected on the command line"
                )
            }
            SessionError::NotFound { id } => write!(f, "no session {id}"),
            SessionError::Internal { message } => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<StoreError> for SessionError {
    fn from(err: StoreError) -> Self {
        SessionError::Store(err)
    }
}

impl From<BackendError> for SessionError {
    fn from(err: BackendError) -> Self {
        SessionError::Backend(err)
    }
}

/// A scenario hint that a backend supplies to the UI: a word to type and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct hint_t {
    pub trigger: String,
    pub description: String,
}

/// What the UI shows about the model and the run.
#[derive(Clone, Debug, PartialEq)]
pub struct runtime_info_t {
    pub version: String,
    pub identity: model_identity_t,
    pub capabilities: capabilities_t,
    pub cwd: String,
    /// The last component of `cwd`.
    pub project: String,
    pub approval_mode: ApprovalMode,
    /// A line the UI shows on every screen when set, such as the demo notice (O1).
    pub notice: Option<String>,
    /// Where state is stored, when it is stored. `None` when nothing is kept.
    pub state_note: Option<String>,
    pub hints: Vec<hint_t>,
}

/// One session. Calls never block. `pump` returns what happened since the last call.
pub trait session_t: Send {
    fn id(&self) -> &str;

    /// Send a message. While a turn runs, the message is queued.
    fn submit(&mut self, text: String);

    /// Stop the running call after the current level pass, cancel a running tool, and drop a
    /// pending approval or question. The queue is kept.
    fn cancel(&mut self);

    fn resolve_approval(&mut self, tool_call_id: &str, decision: ApprovalDecision);

    /// Answer a question. `option_index` is set when the user chose one of the options.
    fn answer_question(&mut self, question_id: &str, answer: String, option_index: Option<u32>);

    /// Messages waiting behind the current turn, oldest first.
    fn queued(&self) -> Vec<String>;

    fn clear_queue(&mut self);

    /// `Full` is refused with `SessionError::FullRequiresFlag`.
    fn set_approval_mode(&mut self, mode: ApprovalMode) -> Result<(), SessionError>;

    fn approval_mode(&self) -> ApprovalMode;

    fn phase(&self) -> SessionPhase;

    /// Events produced since the last call, in order. Never blocks.
    fn pump(&mut self) -> Vec<stamped_event_t>;
}

/// A session read back from the store, with the events to replay into the UI.
pub struct opened_session_t {
    pub session: Box<dyn session_t>,
    pub events: Vec<stamped_event_t>,
    pub skipped_lines: usize,
}

/// Creates and lists sessions.
pub trait runtime_t: Send + Sync {
    fn info(&self) -> &runtime_info_t;

    fn list_sessions(&self) -> Result<Vec<session_summary_t>, SessionError>;

    fn new_session(&self) -> Result<Box<dyn session_t>, SessionError>;

    fn open_session(&self, id: &str) -> Result<opened_session_t, SessionError>;
}
