/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! ratatui front end: layout, mouse, approval and question widgets. This crate depends on
//! `harness` only, and reaches the backend contract types through `harness::backend` (D37).
//!
//! The single entry point is [`run`]. Everything below it is testable without a terminal:
//! the app state machine draws into any ratatui backend, and the terminal guard writes its
//! sequences through a generic writer.

use std::fmt;
use std::io::IsTerminal;
use std::sync::Arc;

use harness::api::runtime_t;

pub mod app;
pub mod banner;
pub mod builtin_themes;
pub mod canvas;
pub mod clipboard;
pub mod commands;
pub mod composer;
pub mod conversation;
pub mod draw;
pub mod effect;
pub mod event_loop;
pub mod folders;
pub mod help;
pub mod input;
pub mod layout;
pub mod markdown;
pub mod menu;
pub mod menu_draw;
pub mod selection;
pub mod settings;
pub mod settings_draw;
pub mod settings_page;
pub mod settings_spec;
pub mod sidebar;
pub mod sidebar_tree;
pub mod strip;
pub mod tab_bar;
pub mod tabs;
pub mod tags;
pub mod terminal;
pub mod text;
pub mod theme;
pub mod theme_actions;
pub mod theme_set;
pub mod transcript;
pub mod view_model;

#[cfg(any(test, feature = "preview"))]
pub mod scripted;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod folder_tests;
#[cfg(test)]
mod select_tests;
#[cfg(test)]
mod settings_tests;
#[cfg(test)]
mod tab_tests;
#[cfg(test)]
mod theme_tests;

pub use theme::ColorMode;

/// How the UI runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct tui_options_t {
    /// The NCM generation effect. `false` shows a static level strip and the final text at once.
    pub animate: bool,
    /// Capture the mouse. `false` leaves the mouse to the terminal; every action keeps a key.
    pub mouse: bool,
    /// `None` reads `COLORTERM`: `truecolor` or `24bit` keep 24-bit colour, else 256 colours.
    pub color: Option<ColorMode>,
}

impl Default for tui_options_t {
    fn default() -> Self {
        tui_options_t {
            animate: true,
            mouse: true,
            color: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiError {
    /// Standard input or output is not a terminal.
    NotATerminal,
    /// A terminal read or write failed.
    Io(String),
    /// The runtime could not start a session.
    Session(String),
}

impl fmt::Display for TuiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TuiError::NotATerminal => write!(
                f,
                "the terminal interface needs a terminal on stdin and stdout"
            ),
            TuiError::Io(message) => write!(f, "terminal i/o error: {message}"),
            TuiError::Session(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for TuiError {}

/// Run the full-screen interface until the user quits. Checks for a terminal before it
/// changes any terminal mode, and restores the terminal on return, on error and on panic.
pub fn run(runtime: Arc<dyn runtime_t>, options: tui_options_t) -> Result<(), TuiError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(TuiError::NotATerminal);
    }
    let app_options = app::app_options_t {
        animate: options.animate,
        mouse: options.mouse,
        color: options.color.unwrap_or_else(theme::detect_color_mode),
    };
    event_loop::run_loop(runtime, app_options)
}
