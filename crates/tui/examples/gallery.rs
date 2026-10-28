/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! UI PREVIEW (scripted events). Runs the real terminal loop against the scripted fixture so
//! the interface can be seen before the controller is wired in. Nothing here is a model or a
//! backend: every reply, tool result and timing is fixed fixture text.
//!
//! Run with `cargo run -p tui --features preview --example gallery`. Quit with `/quit`.

use std::process::ExitCode;
use std::sync::Arc;

use tui::scripted::{preview_info, scripted_runtime_t};
use tui::tui_options_t;

fn main() -> ExitCode {
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "/".to_string());
    let runtime = scripted_runtime_t::preview(preview_info(&cwd));
    let animate = !std::env::args().any(|a| a == "--still");
    let mouse = !std::env::args().any(|a| a == "--no-mouse");
    let options = tui_options_t {
        animate,
        mouse,
        color: None,
    };
    match tui::run(Arc::new(runtime), options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gallery: {err}");
            ExitCode::FAILURE
        }
    }
}
