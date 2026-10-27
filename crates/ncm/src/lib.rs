/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! Export reader, kernels, tokenizers, generator, pick and sampling.
//! This crate depends on nothing in the workspace (PROJECT.md, "Architecture").
//!
//! Today it holds the backend contract ([`backend`]) and the opener for the real engine
//! ([`engine`]), which returns a named not-implemented error.

pub mod backend;
pub mod engine;
