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
pub const Keys: &[(&str, &str)] = &[
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
    ("ctrl+n", "new chat (reuses an unused New chat tab)"),
    (
        "ctrl+t · alt+w",
        "new tab · close the tab (a running turn stops first)",
    ),
    (
        "alt+. · alt+,",
        "next · previous tab (also ctrl+pgdn · ctrl+pgup)",
    ),
    ("alt+1-9 · alt+0", "go to tab 1 to 9 · the last tab"),
    (
        "shift+f10",
        "menu of the sidebar row or selection, else of the tab",
    ),
    ("ctrl+f", "new folder at the top level"),
    (
        "ctrl+o · f2",
        "the settings page (also /settings, sidebar s, ⚙ settings)",
    ),
    ("ctrl+y", "copy the last reply through OSC 52"),
    ("ctrl+x", "clear queued messages"),
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
    ("sidebar: j k enter n", "move · open or fold · new chat"),
    ("sidebar: ← →", "fold · unfold a folder"),
    (
        "sidebar: shift+↑↓ · J K",
        "select from the anchor to the row (extends the selection)",
    ),
    (
        "sidebar: ctrl+space · ctrl+a",
        "add or remove the row · select every row; esc clears",
    ),
    ("sidebar: .", "the menu of the row, or of the selection"),
    ("sidebar: s", "open the settings page"),
    (
        "sidebar: f r p d",
        "new sub-folder · rename · pin · delete (asks first)",
    ),
    (
        "sidebar: m",
        "pick up a row; j k choose a folder, enter drops, esc cancels",
    ),
    ("folder name: enter · esc", "create or rename · cancel"),
    (
        "menu: j k enter → ← esc",
        "choose · run · open a submenu · back · close; 1-9 pick",
    ),
    ("delete: enter · y · n", "delete · delete · keep"),
    (
        "settings: j k · tab",
        "move · section list, rows, then the sidebar",
    ),
    (
        "settings: enter · ← →",
        "change the row · previous or next option; 1-6 a section",
    ),
    (
        "settings: r · d",
        "reset the row to its default · remove an imported theme",
    ),
    (
        "settings: esc",
        "close the import prompt or the question, then the page",
    ),
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

pub const Mouse: &[(&str, &str)] = &[
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
        "click new chat, a folder to fold it, a conversation to open it",
    ),
    ("+ folder", "the button beside conversations: a new folder"),
    (
        "ctrl+click",
        "add or remove a row from the selection (alt+click also)",
    ),
    (
        "shift+click",
        "select from the anchor to the row; ctrl+shift adds the range",
    ),
    (
        "☐ chip",
        "on hover: add or remove the row from the selection",
    ),
    ("right click", "a row, a selection or a tab: its menu"),
    (
        "tabs",
        "click shows · middle click or × closes · + new · wheel scrolls",
    ),
    (
        "drag a row",
        "onto a folder to move it in (a selected row moves the selection)",
    ),
    (
        "row chips",
        "on hover: + sub-folder · ✎ rename · ⇢ move · ✕ delete",
    ),
    ("↓ new chip", "jump to the latest message"),
    (
        "⚙ settings",
        "bottom right of the sidebar: opens the settings page",
    ),
    (
        "settings page",
        "click a section, a chip or a theme; hover a theme to preview it",
    ),
    ("settings tab", "click shows the page · × closes it"),
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
