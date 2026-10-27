/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The backend contract: the one interface between the harness and a model.
//!
//! PROJECT.md, "The model interface" and "Backend contract (P1)", describe it. A call takes a
//! tokenized prefix, sampler settings and a seed. The backend reports progress after each level
//! pass and returns the final token ids. It checks the cancel token between level passes.
//!
//! Every limit a consumer needs is read from [`capabilities_t`]. No consumer assumes a global
//! `P`, `L` or `K`. The real engine (milestone N1), the demo backend (`crates/demo`) and any
//! future adapter to another local runtime implement [`backend_t`].

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Version of this contract. It is written into every session's first event.
pub const ContractVersion: u32 = 1;

/// Which kind of backend answers a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    /// The scripted simulator in `crates/demo`. Its output is not model output.
    Demo,
    /// The NCM engine in this workspace (milestone N1).
    Ncm,
    /// An explicitly configured adapter to another local inference runtime.
    Adapter,
}

impl BackendKind {
    /// True when output is scripted rather than computed by a model.
    pub fn simulated(self) -> bool {
        matches!(self, BackendKind::Demo)
    }

    /// The lowercase name used in logs and in the UI.
    pub fn name(self) -> &'static str {
        match self {
            BackendKind::Demo => "demo",
            BackendKind::Ncm => "ncm",
            BackendKind::Adapter => "adapter",
        }
    }
}

/// Who a backend is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct model_identity_t {
    /// Short model name shown to the user.
    pub id: String,
    pub kind: BackendKind,
    /// Hash of `ncm.json` and the tensor headers (D24). `None` for the demo.
    pub export_hash: Option<String>,
    /// One line that says what this backend is.
    pub note: String,
}

impl model_identity_t {
    pub fn simulated(&self) -> bool {
        self.kind.simulated()
    }
}

/// Sampler settings for one call (PROJECT.md, "NCM facts the runtime depends on", fact 8).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct sampler_t {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
}

impl sampler_t {
    /// Greedy settings, as the XSum and WMT14 configurations use.
    pub fn greedy() -> Self {
        sampler_t {
            temperature: 1.0,
            top_k: 1,
            top_p: 0.0,
        }
    }
}

/// The limits and options of one model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct capabilities_t {
    /// `P`: the longest prefix, in tokens.
    pub prefix_window: u32,
    /// `L`: the number of reply positions every call yields.
    pub reply_len: u32,
    /// `K`: the number of level passes in a call.
    pub level_count: u32,
    /// Positions per level. Length `K`, non-decreasing, and the last entry equals `L`.
    pub level_lens: Vec<u32>,
    /// The end-of-sequence id, when the model has one.
    pub eos_id: Option<u32>,
    pub pad_id: u32,
    pub vocab_size: u32,
    /// Cut the reply at the first EOS (the reference default).
    pub cut_at_eos: bool,
    /// Whether `level_progress_t::draft` is ever set.
    pub supports_drafts: bool,
    pub default_sampler: sampler_t,
}

impl capabilities_t {
    /// Reject an inconsistent set with a named error.
    pub fn validate(&self) -> Result<(), BackendError> {
        let bad = |reason: String| Err(BackendError::InvalidCapabilities { reason });
        if self.prefix_window == 0 {
            return bad("prefix_window (P) is 0".to_string());
        }
        if self.reply_len == 0 {
            return bad("reply_len (L) is 0".to_string());
        }
        if self.level_count == 0 {
            return bad("level_count (K) is 0".to_string());
        }
        if self.level_lens.len() != self.level_count as usize {
            return bad(format!(
                "level_lens has {} entries but level_count is {}",
                self.level_lens.len(),
                self.level_count
            ));
        }
        if self.level_lens.contains(&0) {
            return bad("level_lens holds a zero".to_string());
        }
        if self.level_lens.windows(2).any(|pair| pair[0] > pair[1]) {
            return bad("level_lens is not non-decreasing".to_string());
        }
        if self.level_lens.last() != Some(&self.reply_len) {
            return bad("the last level length is not reply_len (L)".to_string());
        }
        if self.vocab_size == 0 {
            return bad("vocab_size is 0".to_string());
        }
        if self.pad_id >= self.vocab_size {
            return bad("pad_id is outside the vocabulary".to_string());
        }
        if let Some(eos) = self.eos_id
            && eos >= self.vocab_size
        {
            return bad("eos_id is outside the vocabulary".to_string());
        }
        Ok(())
    }
}

/// A shared flag that asks a running call to stop. The engine checks it between level passes.
#[derive(Clone, Debug, Default)]
pub struct cancel_token_t {
    flag: Arc<AtomicBool>,
}

impl cancel_token_t {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the call to stop. Safe to call from any thread, any number of times.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// One engine call.
#[derive(Clone, Debug)]
pub struct call_request_t {
    /// Token ids, 1 to `P` of them.
    pub prefix: Vec<u32>,
    pub sampler: sampler_t,
    pub seed: u64,
    pub cancel: cancel_token_t,
}

impl call_request_t {
    /// The checks every backend applies to a request.
    pub fn validate_for(&self, caps: &capabilities_t) -> Result<(), BackendError> {
        if self.prefix.is_empty() {
            return Err(BackendError::InvalidRequest {
                reason: "the prefix is empty".to_string(),
            });
        }
        if self.prefix.len() > caps.prefix_window as usize {
            return Err(BackendError::PrefixTooLong {
                len: self.prefix.len(),
                window: caps.prefix_window as usize,
            });
        }
        if let Some(bad) = self.prefix.iter().find(|id| **id >= caps.vocab_size) {
            return Err(BackendError::InvalidRequest {
                reason: format!("prefix id {bad} is outside the vocabulary"),
            });
        }
        Ok(())
    }
}

/// Progress after one level pass.
#[derive(Clone, Debug, PartialEq)]
pub struct level_progress_t {
    /// 0-based level index.
    pub level: u32,
    pub level_count: u32,
    /// Positions at this level.
    pub level_len: u32,
    /// The codes so far at this level, `level_len` of them.
    pub codes: Vec<u32>,
    /// Wall time of this pass, measured by the backend.
    pub elapsed: Duration,
    /// Text decoded from the levels so far. Only set when `supports_drafts`.
    pub draft: Option<String>,
}

/// The final block of a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct completion_t {
    /// Exactly `L` ids. The harness cuts at the first EOS when `cut_at_eos` is set.
    pub tokens: Vec<u32>,
}

impl completion_t {
    /// The check the harness applies to what a backend returns.
    pub fn validate_for(&self, caps: &capabilities_t) -> Result<(), BackendError> {
        if self.tokens.len() != caps.reply_len as usize {
            return Err(BackendError::ContractViolation {
                reason: format!(
                    "the backend returned {} ids but reply_len (L) is {}",
                    self.tokens.len(),
                    caps.reply_len
                ),
            });
        }
        if let Some(bad) = self.tokens.iter().find(|id| **id >= caps.vocab_size) {
            return Err(BackendError::ContractViolation {
                reason: format!("the backend returned id {bad}, outside the vocabulary"),
            });
        }
        Ok(())
    }
}

/// A call that stopped between level passes because the cancel token was set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct cancelled_t {
    /// Level passes that finished before the stop.
    pub levels_done: u32,
}

/// How a call that did not fail ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    Completed(completion_t),
    Cancelled(cancelled_t),
}

/// Why a backend call or a backend open failed. Every variant has a stable name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// A feature that this build does not have. `feature` names it.
    NotImplemented {
        feature: String,
    },
    InvalidCapabilities {
        reason: String,
    },
    InvalidRequest {
        reason: String,
    },
    PrefixTooLong {
        len: usize,
        window: usize,
    },
    /// Raised by the harness when a backend breaks the contract.
    ContractViolation {
        reason: String,
    },
    /// A failure inside the backend.
    Failed {
        message: String,
    },
}

impl BackendError {
    /// The stable name that goes into logs.
    pub fn name(&self) -> &'static str {
        match self {
            BackendError::NotImplemented { .. } => "not_implemented",
            BackendError::InvalidCapabilities { .. } => "invalid_capabilities",
            BackendError::InvalidRequest { .. } => "invalid_request",
            BackendError::PrefixTooLong { .. } => "prefix_too_long",
            BackendError::ContractViolation { .. } => "contract_violation",
            BackendError::Failed { .. } => "backend_failed",
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::NotImplemented { feature } => {
                write!(f, "not implemented: {feature}")
            }
            BackendError::InvalidCapabilities { reason } => {
                write!(f, "invalid capabilities: {reason}")
            }
            BackendError::InvalidRequest { reason } => write!(f, "invalid request: {reason}"),
            BackendError::PrefixTooLong { len, window } => {
                write!(
                    f,
                    "the prefix has {len} tokens and the window P is {window}"
                )
            }
            BackendError::ContractViolation { reason } => {
                write!(f, "backend broke the contract: {reason}")
            }
            BackendError::Failed { message } => write!(f, "backend failed: {message}"),
        }
    }
}

impl std::error::Error for BackendError {}

/// A backend's tokenizer. A real model's tokenizer belongs to its family (D18).
pub trait text_codec_t: Send + Sync {
    fn encode(&self, text: &str) -> Vec<u32>;
    fn decode(&self, ids: &[u32]) -> String;
}

/// The model interface. Nothing else in the harness touches an engine.
pub trait backend_t: Send + Sync {
    fn identity(&self) -> &model_identity_t;

    fn capabilities(&self) -> &capabilities_t;

    fn codec(&self) -> &dyn text_codec_t;

    /// Run one call.
    ///
    /// `on_level` runs after each level pass, including the last. The backend checks
    /// `request.cancel` before each pass. If it is set before a pass starts, the call returns
    /// `CallOutcome::Cancelled`. If it is set during the last pass, the call completes. A
    /// backend applies `request.validate_for(self.capabilities())` before it starts.
    fn run_call(
        &self,
        request: call_request_t,
        on_level: &mut dyn FnMut(level_progress_t),
    ) -> Result<CallOutcome, BackendError>;
}
