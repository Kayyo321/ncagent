/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The help overlay: every key, every mouse action, the commands and the demo words.

use harness::api::runtime_info_t;
use ratatui::style::{Modifier, Style};

use crate::commands::Commands;
use crate::text::{line_t, sanitize, span_t, truncate};
use crate::theme::theme_t;

/// Keys, as (key, what it does). The parity test checks the mouse table against these.
pub const Keys: [(&str, &str); 26] = [
    ("enter", "send the message (queued while a turn runs)"),
    ("ctrl+j · alt+enter", "new line in the message"),
    (
        "esc",
        "stop the turn after the current pass; else navigate mode",
    ),
    (
        "ctrl+c",
        "stop the turn; when idle clear input, twice to quit",
    ),
    ("ctrl+d", "quit when the input is empty"),
    ("tab · shift+tab", "move focus: input, transcript, sidebar"),
    ("ctrl+b", "show or hide the sidebar (a drawer when narrow)"),
    ("ctrl+n", "new chat"),
    ("ctrl+y", "copy the last reply through OSC 52"),
    ("ctrl+x", "clear queued messages"),
    ("ctrl+t", "turn the generation effect on or off"),
    ("ctrl+l", "redraw the screen"),
    ("? · f1", "this help (? when the input is empty)"),
    ("pgup · pgdn", "scroll the transcript a page"),
    (
        "ctrl+home · ctrl+end",
        "top of the transcript · jump to latest",
    ),
    ("↑ · ↓", "previous lines of the message, then history"),
    ("ctrl+a · ctrl+e", "start · end of the line"),
    (
        "ctrl+u · ctrl+k · ctrl+w",
        "delete to start · to end · word",
    ),
    ("alt+← · alt+→", "move by word"),
    ("navigate: j k ↑ ↓", "select a block in the transcript"),
    (
        "navigate: enter · space",
        "expand or fold the selected tool or attempt",
    ),
    ("navigate: g · G · i", "top · latest · back to the input"),
    ("sidebar: j k enter n", "move · open · new chat"),
    (
        "approval: y a n",
        "allow once · allow for the session · deny",
    ),
    ("approval: ← → enter", "choose a button and press it"),
    (
        "question: 1-9 · enter",
        "pick an option · send the typed answer",
    ),
];

pub const Mouse: [(&str, &str); 9] = [
    (
        "wheel",
        "scroll the transcript or the sidebar under the pointer",
    ),
    (
        "scrollbar",
        "press or drag the right edge of the transcript",
    ),
    ("tool block", "click to expand or fold its output"),
    ("level strip", "click to stop after the current pass"),
    ("approval buttons", "click once, session or deny"),
    ("question options", "click an option"),
    (
        "sidebar",
        "click new chat, a project to fold it, a conversation to open it",
    ),
    ("↓ new chip", "jump to the latest message"),
    (
        "shift+drag",
        "select text natively; the UI owns plain drags",
    ),
];

fn row(key: &str, about: &str, theme: &theme_t) -> line_t {
    line_t::new(vec![
        span_t::new(
            format!("  {key:<24}"),
            Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        span_t::new(about.to_string(), Style::new().fg(theme.fg_dim)),
    ])
}

fn title(text: &str, theme: &theme_t) -> line_t {
    line_t::plain(
        text,
        Style::new().fg(theme.ice).add_modifier(Modifier::BOLD),
    )
}

pub fn help_lines(info: &runtime_info_t, mouse: bool, theme: &theme_t) -> Vec<line_t> {
    let mut out = vec![title("keys", theme)];
    for (k, a) in Keys {
        out.push(row(k, a, theme));
    }
    out.push(line_t::blank());
    out.push(title(
        if mouse {
            "mouse"
        } else {
            "mouse (off in this run)"
        },
        theme,
    ));
    for (k, a) in Mouse {
        out.push(row(k, a, theme));
    }
    out.push(line_t::blank());
    out.push(title("commands", theme));
    for c in Commands {
        let name = if c.args.is_empty() {
            c.name.to_string()
        } else {
            format!("{} {}", c.name, c.args)
        };
        out.push(row(&name, c.about, theme));
    }
    out.push(row(
        "/approval full",
        "refused here: full is set on the command line only",
        theme,
    ));
    if !info.hints.is_empty() {
        out.push(line_t::blank());
        out.push(title("try typing", theme));
        for h in &info.hints {
            out.push(row(
                &truncate(&sanitize(&h.trigger, false), 22),
                &sanitize(&h.description, false),
                theme,
            ));
        }
    }
    out.push(line_t::blank());
    out.push(line_t::plain(
        "  any key closes this help",
        Style::new()
            .fg(theme.fg_faint)
            .add_modifier(Modifier::ITALIC),
    ));
    out
}
