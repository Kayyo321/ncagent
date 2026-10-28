/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! Sessions, event log, packing, reply protocol, tools, approvals, compaction,
//! sub-agents, side chats and instructions. No terminal I/O lives here, and this
//! crate never depends on `convert`, `tui` or `demo` (PROJECT.md, "Architecture").
//!
//! The backend contract is re-exported as [`backend`] so that `tui` never names `ncm` (D37).

pub use ncm::backend;

pub mod api;
pub mod approval;
pub mod controller;
pub mod event;
pub mod organize;
pub mod pack;
pub mod replay;
pub mod reply;
pub mod runtime;
pub mod settings;
pub mod store;
pub mod tools;
