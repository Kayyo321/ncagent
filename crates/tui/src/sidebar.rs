/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The right-hand column: the user's way around the UI. A new-chat button, the state of the
//! current turn and call, the model, and the conversations as a tree of folders (D54). Sections
//! are a bold label over dim values, separated by space, never by lines. Nesting is shown by
//! indentation and faint guide rails, and a move (drag or pick-up, D55) by tone steps.

use harness::api::{SessionPhase, runtime_info_t};
use harness::organize::TagColor;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::banner::{chip, short_path};
use crate::canvas::{fill, put_line, put_str, put_str_right};
use crate::input::{Action, MenuAction, SelectAction, Target, hit_registry_t};
use crate::settings_page::PageAction;
use crate::strip::strip_line;
use crate::tags::{CheckOff, CheckOn, PinGlyph, TagDot, tag_rgb};
use crate::text::{
    clusters, format_ms, group_digits, line_t, sanitize, str_width, truncate, wrap_plain,
};
use crate::theme::{mix, theme_t};
use crate::view_model::transcript_t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Generating,
    /// Waiting for the user: an approval or a question.
    Waiting,
    /// Running a tool.
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarItemKind {
    NewChat,
    /// The "conversations" label: the top level of the tree, and a drop target (D55).
    Root,
    Folder {
        id: String,
        collapsed: bool,
    },
    Session {
        id: String,
    },
}

/// How a row shows while a conversation or folder is being moved (D55).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveMark {
    None,
    /// The row being moved.
    Source,
    /// A destination the move may use.
    Allowed,
    /// A destination the move may not use, with a short reason.
    Refused {
        reason: String,
    },
    /// Not a destination: a conversation row while something moves.
    Inert,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct sidebar_item_t {
    pub kind: SidebarItemKind,
    pub title: String,
    pub detail: String,
    pub active: bool,
    pub activity: Activity,
    pub demo: bool,
    /// Nesting level in the tree: 0 for top-level folders and unfiled conversations.
    pub depth: usize,
    pub mark: MoveMark,
    /// A conversation's own settings (D57).
    pub pinned: bool,
    pub color: Option<TagColor>,
    pub archived: bool,
    /// In a selection that shows (D60).
    pub selected: bool,
}

impl sidebar_item_t {
    /// A row with this kind and title and every other field at its plain value.
    pub fn label(kind: SidebarItemKind, title: String) -> Self {
        sidebar_item_t {
            kind,
            title,
            detail: String::new(),
            active: false,
            activity: Activity::Idle,
            demo: false,
            depth: 0,
            mark: MoveMark::None,
            pinned: false,
            color: None,
            archived: false,
            selected: false,
        }
    }

    /// A row of the scrolled tree (not the new-chat button or the "conversations" label).
    pub fn in_tree(&self) -> bool {
        matches!(
            self.kind,
            SidebarItemKind::Folder { .. } | SidebarItemKind::Session { .. }
        )
    }
}

/// Rows the sidebar inserts into the tree: the folder-name prompt or the delete confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    Prompt {
        caption: String,
        text: String,
        /// Byte offset of the cursor in `text`.
        cursor: usize,
    },
    Confirm {
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct overlay_t {
    pub kind: OverlayKind,
    /// The item the rows follow. `None` puts them at the top of the tree.
    pub anchor: Option<usize>,
    /// The rows take the place of the anchor row (a rename).
    pub replace: bool,
    pub depth: usize,
}

pub struct sidebar_input_t<'a> {
    pub info: &'a runtime_info_t,
    pub transcript: &'a transcript_t,
    pub phase: SessionPhase,
    pub phase_text: &'a str,
    pub items: &'a [sidebar_item_t],
    pub selected: usize,
    pub focused: bool,
    pub scroll: usize,
    pub clock: u64,
    pub animate: bool,
    pub hover: Option<(u16, u16)>,
    pub drawer: bool,
    /// Keep the selected row in view. Off while a drag scrolls the tree.
    pub follow: bool,
    /// The destination a move would use now: the row under the pointer or the pick-up choice.
    pub hot: Option<usize>,
    /// The title being dragged, shown as a ghost label at the pointer.
    pub ghost: Option<String>,
    pub overlay: Option<&'a overlay_t>,
    /// Show the row chips (select, new sub-folder, rename, move, delete) on the hovered or
    /// cursor row.
    pub chips: bool,
    /// Rows in the selection, when it shows. Two or more get the selection line (D60).
    pub selected_count: usize,
    /// Which sections show, by the `show_*` settings (D63).
    pub sections: sidebar_sections_t,
    /// The compact density: no blank rows between the sections (D63).
    pub compact: bool,
    /// The settings page shows, so its footer button is lit (D65).
    pub page_shown: bool,
}

/// The sections above the conversation list that show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct sidebar_sections_t {
    pub context: bool,
    pub turn: bool,
    pub generation: bool,
    pub model: bool,
}

impl Default for sidebar_sections_t {
    fn default() -> Self {
        sidebar_sections_t {
            context: true,
            turn: true,
            generation: true,
            model: true,
        }
    }
}

/// The footer button that opens the settings page (D65).
pub const SettingsButton: &str = " ⚙ settings ";

/// What drawing the sidebar decided.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct sidebar_out_t {
    pub scroll: usize,
    /// The rows of the tree, for drag auto-scroll.
    pub list: Rect,
}

struct srow_t {
    line: line_t,
    right: Option<line_t>,
    target: Option<Action>,
    bg: Option<Color>,
}

impl srow_t {
    fn text(line: line_t) -> Self {
        srow_t {
            line,
            right: None,
            target: None,
            bg: None,
        }
    }

    fn pair(left: line_t, right: line_t) -> Self {
        srow_t {
            line: left,
            right: Some(right),
            target: None,
            bg: None,
        }
    }
}

fn label(text: &str, theme: &theme_t) -> line_t {
    line_t::plain(text, Style::new().fg(theme.fg).add_modifier(Modifier::BOLD))
}

fn dim(text: impl Into<String>, theme: &theme_t) -> line_t {
    line_t::plain(text, Style::new().fg(theme.fg_dim))
}

fn faint(text: impl Into<String>, theme: &theme_t) -> line_t {
    line_t::plain(text, Style::new().fg(theme.fg_faint))
}

/// Sections above the conversation list, highest priority first. Lower sections are dropped
/// when the column is short.
fn sections(input: &sidebar_input_t<'_>, inner: usize, theme: &theme_t) -> Vec<(u8, Vec<srow_t>)> {
    let t = input.transcript;
    let info = input.info;
    let caps = &info.capabilities;
    let sim = info.identity.simulated();
    let mut out = Vec::new();

    // Context.
    let mut rows = Vec::new();
    match t.context {
        Some(c) if c.window > 0 => {
            let pct = (u64::from(c.packed_tokens) * 100 / u64::from(c.window)).min(999);
            rows.push(srow_t::pair(
                label("context", theme),
                dim(format!("{pct}%"), theme),
            ));
            rows.push(srow_t::text(dim(
                format!(
                    "{} / {} tokens",
                    group_digits(u64::from(c.packed_tokens)),
                    group_digits(u64::from(c.window))
                ),
                theme,
            )));
            let filled = (inner as u64 * u64::from(c.packed_tokens.min(c.window))
                / u64::from(c.window)) as usize;
            let mut gauge = line_t::blank();
            gauge.push("▀".repeat(filled), Style::new().fg(theme.agent));
            gauge.push(
                "▀".repeat(inner.saturating_sub(filled)),
                Style::new().fg(theme.inset),
            );
            rows.push(srow_t::text(gauge));
            if c.dropped_turns > 0 {
                rows.push(srow_t::text(faint(
                    format!("{} older turn(s) left out of the window", c.dropped_turns),
                    theme,
                )));
            }
        }
        _ => {
            rows.push(srow_t::pair(label("context", theme), faint("—", theme)));
            rows.push(srow_t::text(faint(
                format!(
                    "window P = {} tokens",
                    group_digits(u64::from(caps.prefix_window))
                ),
                theme,
            )));
        }
    }
    if input.sections.context {
        out.push((0, rows));
    }

    // Turn.
    let mut rows = Vec::new();
    let (dot, color) = match input.phase {
        SessionPhase::Idle => ("●", theme.success),
        SessionPhase::AwaitingApproval | SessionPhase::AwaitingAnswer => ("●", theme.approval),
        SessionPhase::RunningTool => ("●", theme.tool),
        SessionPhase::Cancelling => ("◌", theme.approval),
        SessionPhase::Generating => ("●", theme.ice),
    };
    let mut right = line_t::blank();
    right.push(dot, Style::new().fg(color));
    rows.push(srow_t::pair(label("turn", theme), right));
    let turn = t.turn;
    let turn_text = if turn.turn == 0 {
        "no turn yet".to_string()
    } else {
        format!("turn {} · {}", turn.turn, input.phase_text)
    };
    rows.push(srow_t::text(dim(truncate(&turn_text, inner), theme)));
    if turn.turn > 0 {
        rows.push(srow_t::text(faint(
            format!("{} call(s) · {} level pass(es)", turn.calls, turn.passes),
            theme,
        )));
    }
    if input.sections.turn {
        out.push((0, rows));
    }

    // Generation.
    let mut rows = Vec::new();
    let mut right = line_t::blank();
    if sim {
        right.push("simulated", Style::new().fg(theme.approval));
    }
    rows.push(srow_t::pair(label("generation", theme), right));
    match t.latest_reply() {
        Some(reply) => {
            if let Some(a) = reply.last() {
                rows.push(srow_t::text(dim(
                    truncate(
                        &format!(
                            "call {} · attempt {} · seed {}",
                            reply.call.saturating_add(1),
                            a.attempt.saturating_add(1),
                            a.seed
                        ),
                        inner,
                    ),
                    theme,
                )));
                let strip = strip_line(
                    &a.levels,
                    a.level_count,
                    a.running(),
                    input.clock,
                    input.animate,
                    inner,
                    theme,
                );
                rows.push(srow_t {
                    line: strip,
                    right: None,
                    target: a.running().then_some(Action::Cancel),
                    bg: None,
                });
                let lens: Vec<String> = a.levels.iter().map(|l| l.level_len.to_string()).collect();
                if !lens.is_empty() {
                    rows.push(srow_t::text(faint(
                        truncate(&format!("level_len {}", lens.join(" · ")), inner),
                        theme,
                    )));
                }
                let state = if a.running() {
                    format!("{} of {} levels done", a.levels.len(), a.level_count)
                } else {
                    format!(
                        "{} over {} pass(es)",
                        format_ms(a.elapsed_ms()),
                        a.levels.len()
                    )
                };
                rows.push(srow_t::text(faint(truncate(&state, inner), theme)));
            }
        }
        None => rows.push(srow_t::text(faint("no call yet", theme))),
    }
    rows.push(srow_t::text(faint(
        truncate(
            &format!(
                "P {} · L {} · K {}",
                caps.prefix_window, caps.reply_len, caps.level_count
            ),
            inner,
        ),
        theme,
    )));
    let drafts = if caps.supports_drafts {
        "drafts shape the blocks"
    } else {
        "no drafts: blocks follow level_len"
    };
    rows.push(srow_t::text(faint(truncate(drafts, inner), theme)));
    rows.push(srow_t::text(faint(
        if sim {
            "times: simulator wall clock"
        } else {
            "times: measured wall clock"
        },
        theme,
    )));
    if input.sections.generation {
        out.push((1, rows));
    }

    // Model.
    let mut rows = Vec::new();
    let mut right = line_t::blank();
    if sim {
        right.spans.push(chip("DEMO", theme.bg, theme.approval));
    }
    rows.push(srow_t::pair(label("model", theme), right));
    rows.push(srow_t::text(dim(
        truncate(&sanitize(&info.identity.id, false), inner),
        theme,
    )));
    let contract = t
        .meta
        .as_ref()
        .map(|m| format!(" · contract v{}", m.contract_version))
        .unwrap_or_default();
    let kind = format!("{} backend{contract}", info.identity.kind.name());
    rows.push(srow_t::text(faint(truncate(&kind, inner), theme)));
    for l in crate::text::wrap_plain(
        &sanitize(&info.identity.note, false),
        inner,
        Style::new().fg(theme.fg_faint),
    )
    .into_iter()
    .take(2)
    {
        rows.push(srow_t::text(l));
    }
    if input.sections.model {
        out.push((2, rows));
    }
    out
}

/// One row of the drawn tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum disp_t {
    Item(usize),
    Overlay(usize),
}

/// One row of an overlay: the text, its tone, an optional rail, and clickable spans as
/// (column offset, width, action).
struct overlay_row_t {
    line: line_t,
    bg: Color,
    rail: Option<Color>,
    buttons: Vec<(u16, u16, Action)>,
}

/// The tone of a destination under the pointer or the pick-up choice.
fn hot_bg(bg: Color, theme: &theme_t) -> Color {
    mix(bg, theme.agent, 0.24)
}

fn refused_bg(bg: Color, theme: &theme_t) -> Color {
    mix(bg, theme.error, 0.18)
}

fn confirm_bg(bg: Color, theme: &theme_t) -> Color {
    mix(bg, theme.error, 0.12)
}

/// The end of `text` that fits `width` columns.
fn tail(text: &str, width: usize) -> String {
    let mut used = 0usize;
    let mut start = text.len();
    for c in clusters(text).iter().rev() {
        if used + c.width > width {
            break;
        }
        used += c.width;
        start = c.start;
    }
    text.get(start..).unwrap_or("").to_string()
}

/// A one-line editor: the text around a block cursor, scrolled so the cursor stays in view.
pub(crate) fn prompt_line(
    text: &str,
    cursor: usize,
    width: usize,
    bg: Color,
    theme: &theme_t,
) -> line_t {
    let cursor = cursor.min(text.len());
    let before = sanitize(text.get(..cursor).unwrap_or(""), false);
    let after = sanitize(text.get(cursor..).unwrap_or(""), false);
    let first = clusters(&after).first().map(|c| (c.start, c.end));
    let (under, rest) = match first {
        Some((a, b)) => (
            after.get(a..b).unwrap_or(" ").to_string(),
            after.get(b..).unwrap_or("").to_string(),
        ),
        None => (" ".to_string(), String::new()),
    };
    let room = width.saturating_sub(str_width(&under));
    let shown = tail(&before, room);
    let left = room.saturating_sub(str_width(&shown));
    let style = Style::new().fg(theme.fg).bg(bg);
    let mut line = line_t::plain(shown, style);
    line.push(under, Style::new().fg(theme.bg).bg(theme.ice));
    if left > 0 {
        line.push(truncate(&rest, left), style);
    }
    line
}

fn overlay_rows(
    overlay: &overlay_t,
    width: usize,
    bg: Color,
    theme: &theme_t,
) -> Vec<overlay_row_t> {
    let width = width.max(4);
    match &overlay.kind {
        OverlayKind::Prompt {
            caption,
            text,
            cursor,
        } => {
            let caption = line_t::plain(
                truncate(caption, width),
                Style::new().fg(theme.fg_dim).add_modifier(Modifier::ITALIC),
            );
            let field = theme.inset;
            let mut input = line_t::plain("▸ ", Style::new().fg(theme.ice).bg(field));
            for span in prompt_line(text, *cursor, width.saturating_sub(2), field, theme).spans {
                input.push(span.text, span.style);
            }
            vec![
                overlay_row_t {
                    line: caption,
                    bg,
                    rail: None,
                    buttons: Vec::new(),
                },
                overlay_row_t {
                    line: input,
                    bg: field,
                    rail: Some(theme.ice),
                    buttons: Vec::new(),
                },
            ]
        }
        OverlayKind::Confirm { name } => {
            let tone = confirm_bg(bg, theme);
            let mut rows: Vec<overlay_row_t> = wrap_plain(
                &format!("delete folder “{name}”? its contents move up"),
                width,
                Style::new().fg(theme.fg),
            )
            .into_iter()
            .take(3)
            .map(|line| overlay_row_t {
                line,
                bg: tone,
                rail: Some(theme.error),
                buttons: Vec::new(),
            })
            .collect();
            let yes = " enter confirm ";
            let no = " esc cancel ";
            let mut line = line_t::plain(
                yes,
                Style::new()
                    .fg(theme.bg)
                    .bg(theme.error)
                    .add_modifier(Modifier::BOLD),
            );
            line.push(" ", Style::new());
            line.push(no, Style::new().fg(theme.fg).bg(theme.inset));
            let yes_w = str_width(yes) as u16;
            let no_w = str_width(no) as u16;
            rows.push(overlay_row_t {
                line,
                bg: tone,
                rail: Some(theme.error),
                buttons: vec![
                    (0, yes_w, Action::ConfirmYes),
                    (yes_w + 1, no_w, Action::SidebarCancel),
                ],
            });
            rows
        }
    }
}

/// The tree as drawn rows: every item but the button and the label, with the overlay rows
/// placed after (or instead of) their anchor.
fn tree_rows(input: &sidebar_input_t<'_>, overlay_len: usize) -> Vec<disp_t> {
    let mut out = Vec::with_capacity(input.items.len() + overlay_len);
    let overlay = input.overlay;
    if overlay.is_some_and(|o| o.anchor.is_none()) {
        out.extend((0..overlay_len).map(disp_t::Overlay));
    }
    for (index, item) in input.items.iter().enumerate() {
        if !item.in_tree() {
            continue;
        }
        let anchored = overlay.is_some_and(|o| o.anchor == Some(index));
        let replaced = anchored && overlay.is_some_and(|o| o.replace);
        if !replaced {
            out.push(disp_t::Item(index));
        }
        if anchored {
            out.extend((0..overlay_len).map(disp_t::Overlay));
        }
    }
    out
}

/// Draw the sidebar into `area`.
pub fn draw_sidebar(
    buf: &mut Buffer,
    area: Rect,
    input: &sidebar_input_t<'_>,
    theme: &theme_t,
    hits: &mut hit_registry_t,
) -> sidebar_out_t {
    let mut out = sidebar_out_t {
        scroll: input.scroll,
        list: Rect::default(),
    };
    if area.width < 8 || area.height < 4 {
        return out;
    }
    let bg = if input.drawer { theme.dock } else { theme.side };
    fill(buf, area, Style::new().bg(bg).fg(theme.fg));
    let x0 = area.x.saturating_add(2);
    let x1 = area.right().saturating_sub(2);
    let inner = x1.saturating_sub(x0) as usize;
    let base = Style::new().bg(bg);
    let mut y = area.y.saturating_add(1);
    let bottom = area.bottom();
    // Blank rows between the parts of the column: none with the compact density.
    let gap = u16::from(!input.compact);

    // Title.
    put_str(
        buf,
        x0,
        y,
        x1,
        "ncagent",
        base.fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    if input.info.identity.simulated() {
        put_str_right(
            buf,
            x1,
            y,
            x0,
            " DEMO ",
            Style::new()
                .fg(theme.bg)
                .bg(theme.approval)
                .add_modifier(Modifier::BOLD),
        );
    }
    y = y.saturating_add(1 + gap);

    // New chat button.
    let button = Rect::new(x0, y, x1.saturating_sub(x0), 1);
    let new_selected = input.focused
        && matches!(
            input.items.get(input.selected).map(|i| &i.kind),
            Some(SidebarItemKind::NewChat)
        );
    let hovered = input
        .hover
        .is_some_and(|(hx, hy)| button.contains(Position::new(hx, hy)));
    let button_bg = if new_selected || hovered {
        theme.select
    } else {
        theme.inset
    };
    fill(buf, button, Style::new().bg(button_bg));
    let after = put_str(
        buf,
        x0.saturating_add(1),
        y,
        x1,
        "+",
        Style::new()
            .fg(theme.ice)
            .bg(button_bg)
            .add_modifier(Modifier::BOLD),
    );
    put_str(
        buf,
        after.saturating_add(1),
        y,
        x1,
        "New chat",
        Style::new().fg(theme.fg).bg(button_bg),
    );
    put_str_right(
        buf,
        x1.saturating_sub(1),
        y,
        x0,
        "ctrl+n",
        Style::new().fg(theme.fg_faint).bg(button_bg),
    );
    hits.add(button, Action::NewChat);
    y = y.saturating_add(1 + gap);

    // Footer: cwd and version, at the very bottom.
    let footer_h = 3u16;
    let footer_y = bottom.saturating_sub(footer_h);

    // Fit sections above the conversations; drop low-priority ones when short.
    let mut secs = sections(input, inner, theme);
    let min_list = 6usize;
    loop {
        let need: usize = secs.iter().map(|(_, r)| r.len() + usize::from(gap)).sum();
        let avail = footer_y.saturating_sub(y) as usize;
        if need + min_list <= avail || secs.len() <= 1 {
            break;
        }
        let worst = secs.iter().map(|(p, _)| *p).max().unwrap_or(0);
        match secs.iter().rposition(|(p, _)| *p == worst) {
            Some(i) => {
                secs.remove(i);
            }
            None => break,
        }
    }
    for (_, rows) in secs {
        for row in rows {
            if y >= footer_y {
                break;
            }
            let rect = Rect::new(x0, y, x1.saturating_sub(x0), 1);
            if let Some(row_bg) = row.bg {
                fill(buf, rect, Style::new().bg(row_bg));
            }
            put_line(buf, x0, y, x1, &row.line, base);
            if let Some(right) = &row.right {
                let w = right.width() as u16;
                put_line(buf, x1.saturating_sub(w).max(x0), y, x1, right, base);
            }
            if let Some(t) = row.target {
                hits.add(rect, t);
            }
            y = y.saturating_add(1);
        }
        y = y.saturating_add(gap);
    }

    // Conversations.
    if y < footer_y {
        let ctx = tree_ctx_t {
            area,
            bg,
            input,
            moving: input.items.iter().any(|i| i.mark != MoveMark::None),
            theme,
        };
        draw_root(buf, &ctx, y, hits);
        y = y.saturating_add(1);
        if input.selected_count > 1 && y < footer_y.saturating_sub(2) {
            draw_selection_line(buf, &ctx, y, hits);
            y = y.saturating_add(1);
        }
        let overlay = input
            .overlay
            .map(|o| {
                let indent = (o.depth * 2) as u16;
                let width = x1.saturating_sub(x0.saturating_add(indent)) as usize;
                overlay_rows(o, width, bg, theme)
            })
            .unwrap_or_default();
        let rows_list = tree_rows(input, overlay.len());
        let rows = footer_y.saturating_sub(y).saturating_sub(1) as usize;
        out.list = Rect::new(area.x, y, area.width, rows as u16);
        if rows > 0 {
            let want = if overlay.is_empty() {
                input
                    .follow
                    .then(|| {
                        rows_list
                            .iter()
                            .position(|d| *d == disp_t::Item(input.selected))
                    })
                    .flatten()
                    .map(|p| (p, p))
            } else {
                let first = rows_list
                    .iter()
                    .position(|d| matches!(d, disp_t::Overlay(_)));
                let last = rows_list
                    .iter()
                    .rposition(|d| matches!(d, disp_t::Overlay(_)));
                first.zip(last)
            };
            let mut scroll = input.scroll;
            if let Some((lo, hi)) = want {
                if lo < scroll {
                    scroll = lo;
                } else if hi >= scroll + rows {
                    scroll = (hi + 1).saturating_sub(rows).min(lo);
                }
            }
            scroll = scroll.min(rows_list.len().saturating_sub(rows));
            out.scroll = scroll;
            for d in rows_list.iter().skip(scroll).take(rows) {
                match *d {
                    disp_t::Item(index) => {
                        if let Some(item) = input.items.get(index) {
                            draw_tree_row(buf, &ctx, y, index, item, hits);
                        }
                    }
                    disp_t::Overlay(k) => {
                        if let (Some(row), Some(o)) = (overlay.get(k), input.overlay) {
                            draw_overlay_row(buf, area, y, row, o.depth, theme, hits);
                        }
                    }
                }
                y = y.saturating_add(1);
            }
            if rows_list.len() > scroll + rows {
                put_str(
                    buf,
                    x0,
                    y,
                    x1,
                    &format!("↓ {} more", rows_list.len() - scroll - rows),
                    base.fg(theme.fg_faint),
                );
            }
        }
    }

    // Footer.
    let fy = footer_y.saturating_add(1);
    let cwd = sanitize(&input.info.cwd, false);
    let (dir, last) = match cwd.trim_end_matches('/').rsplit_once('/') {
        Some((d, l)) => (format!("{d}/"), l.to_string()),
        None => (String::new(), cwd.clone()),
    };
    let last_w = str_width(&last).min(inner);
    let dir_short = short_path(&dir, inner.saturating_sub(last_w));
    let nx = put_str(buf, x0, fy, x1, &dir_short, base.fg(theme.fg_faint));
    put_str(
        buf,
        nx,
        fy,
        x1,
        &truncate(&last, inner),
        base.fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    let version = format!(
        "ncagent {}{}",
        truncate(&sanitize(&input.info.version, false), 12),
        if input.info.identity.simulated() {
            " · demo"
        } else {
            ""
        }
    );
    // The settings button sits at the right of the version row; the version gives way first.
    let vy = fy.saturating_add(1);
    let button_w = str_width(SettingsButton) as u16;
    let bx = x1.saturating_sub(button_w).max(x0);
    let button = Rect::new(bx, vy, x1.saturating_sub(bx), 1);
    let over = input
        .hover
        .is_some_and(|(hx, hy)| button.contains(Position::new(hx, hy)));
    let (button_fg, button_bg) = match (input.page_shown, over) {
        (_, true) => (theme.fg, theme.select),
        (true, false) => (theme.fg, theme.hover),
        (false, false) => (theme.fg_dim, theme.inset),
    };
    let gear_x = put_str(
        buf,
        bx,
        vy,
        x1,
        " ⚙",
        Style::new()
            .fg(if input.page_shown {
                theme.ice
            } else {
                button_fg
            })
            .bg(button_bg),
    );
    put_str(
        buf,
        gear_x,
        vy,
        x1,
        " settings ",
        Style::new().fg(button_fg).bg(button_bg),
    );
    hits.add(button, Action::Page(PageAction::Open));
    let room = usize::from(bx.saturating_sub(x0).saturating_sub(1));
    // " · demo" goes first, then the name, before the version number is cut: the DEMO chip at
    // the top stays, and the title says ncagent.
    let number = truncate(&sanitize(&input.info.version, false), 12);
    let version = [version, format!("ncagent {number}"), format!("v{number}")]
        .into_iter()
        .find(|v| str_width(v) <= room)
        .unwrap_or_else(|| format!("v{number}"));
    put_str(
        buf,
        x0,
        vy,
        bx.saturating_sub(1),
        &truncate(&version, room),
        base.fg(theme.fg_faint),
    );

    // The ghost of a dragged row follows the pointer, one row below it so the destination
    // stays readable.
    if let (Some(title), Some((px, py))) = (&input.ghost, input.hover)
        && area.contains(Position::new(px, py))
    {
        let text = format!(
            " ⇢ {} ",
            truncate(title, usize::from(area.width).saturating_sub(8))
        );
        let w = str_width(&text) as u16;
        let gy = if py.saturating_add(1) < area.bottom() {
            py.saturating_add(1)
        } else {
            py.saturating_sub(1)
        };
        let gx = px
            .saturating_add(2)
            .min(area.right().saturating_sub(w))
            .max(area.x);
        put_str(
            buf,
            gx,
            gy,
            area.right(),
            &text,
            Style::new()
                .fg(theme.fg)
                .bg(theme.select)
                .add_modifier(Modifier::BOLD),
        );
    }
    out
}

/// What every row of the tree draws against.
struct tree_ctx_t<'a, 'b> {
    area: Rect,
    bg: Color,
    input: &'a sidebar_input_t<'b>,
    /// A move is in progress, so rows show their move marks instead of chips.
    moving: bool,
    theme: &'a theme_t,
}

/// The "conversations" label: the top level of the tree. It holds the new-folder button, and
/// during a move it is the destination "top level".
fn draw_root(buf: &mut Buffer, ctx: &tree_ctx_t<'_, '_>, y: u16, hits: &mut hit_registry_t) {
    let (area, bg, input, moving, theme) = (ctx.area, ctx.bg, ctx.input, ctx.moving, ctx.theme);
    let x0 = area.x.saturating_add(2);
    let x1 = area.right().saturating_sub(2);
    let rect = Rect::new(area.x, y, area.width, 1);
    let index = input
        .items
        .iter()
        .position(|i| i.kind == SidebarItemKind::Root);
    let item = index.and_then(|i| input.items.get(i));
    let hot = index.is_some() && input.hot == index;
    let refused = item.is_some_and(|i| matches!(i.mark, MoveMark::Refused { .. }));
    let row_bg = match (hot, refused) {
        (true, false) => hot_bg(bg, theme),
        (true, true) => refused_bg(bg, theme),
        _ => bg,
    };
    fill(buf, rect, Style::new().bg(row_bg));
    let st = Style::new().bg(row_bg);
    if hot {
        let rail = if refused { theme.error } else { theme.ice };
        put_str(buf, area.x, y, area.right(), "▌", st.fg(rail));
    }
    let label_fg = if moving && refused && !hot {
        theme.fg_faint
    } else {
        theme.fg
    };
    let after = put_str(
        buf,
        x0,
        y,
        x1,
        "conversations",
        st.fg(label_fg).add_modifier(Modifier::BOLD),
    );
    if let Some(i) = index {
        hits.add_target(rect, Target::Row(i));
    }
    if moving {
        let (text, fg) = match item.map(|i| &i.mark) {
            Some(MoveMark::Refused { reason }) => (reason.clone(), theme.error),
            _ if hot => ("↑ top level".to_string(), theme.ice),
            _ => ("↑ top level".to_string(), theme.fg_faint),
        };
        put_str_right(buf, x1, y, after.saturating_add(1), &text, st.fg(fg));
        return;
    }
    // The new-folder button.
    let label = " + folder ";
    let w = str_width(label) as u16;
    let bx = x1.saturating_sub(w);
    if bx > after {
        let button = Rect::new(bx, y, w, 1);
        let over = input
            .hover
            .is_some_and(|(hx, hy)| button.contains(Position::new(hx, hy)));
        let chip_bg = if over { theme.select } else { theme.inset };
        let nx = put_str(
            buf,
            bx,
            y,
            x1,
            " +",
            Style::new()
                .fg(theme.ice)
                .bg(chip_bg)
                .add_modifier(Modifier::BOLD),
        );
        put_str(
            buf,
            nx,
            y,
            x1,
            " folder ",
            Style::new()
                .fg(if over { theme.fg } else { theme.fg_dim })
                .bg(chip_bg),
        );
        hits.add(button, Action::NewFolder);
    }
}

/// The line under "conversations" while several rows are selected: how many, a chip for their
/// menu and a chip that clears the selection (D60, D61).
fn draw_selection_line(
    buf: &mut Buffer,
    ctx: &tree_ctx_t<'_, '_>,
    y: u16,
    hits: &mut hit_registry_t,
) {
    let (area, input, theme) = (ctx.area, ctx.input, ctx.theme);
    let x0 = area.x.saturating_add(2);
    let x1 = area.right().saturating_sub(2);
    let rect = Rect::new(area.x, y, area.width, 1);
    let tone = theme.marked;
    fill(buf, rect, Style::new().bg(tone));
    let st = Style::new().bg(tone);
    put_str(buf, area.x, y, area.right(), "▎", st.fg(theme.ice));
    let mut cx = x1;
    for (glyph, action) in [
        (" ✕ ", Action::Select(SelectAction::Clear)),
        (" ⋯ ", Action::Menu(MenuAction::OpenSelection)),
    ] {
        cx = cx.saturating_sub(3);
        let chip = Rect::new(cx, y, 3, 1);
        let over = input
            .hover
            .is_some_and(|(hx, hy)| chip.contains(Position::new(hx, hy)));
        let (fg, chip_bg) = if over {
            (theme.fg, theme.select)
        } else {
            (theme.fg_dim, mix(tone, theme.fg, 0.08))
        };
        put_str(buf, cx, y, x1, glyph, Style::new().fg(fg).bg(chip_bg));
        hits.add(chip, action);
    }
    let text = format!("{} selected", input.selected_count);
    let nx = put_str(
        buf,
        x0,
        y,
        cx.saturating_sub(1),
        &text,
        st.fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    put_str(
        buf,
        nx.saturating_add(1),
        y,
        cx.saturating_sub(1),
        "esc clears",
        st.fg(theme.fg_faint),
    );
}

fn draw_overlay_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    row: &overlay_row_t,
    depth: usize,
    theme: &theme_t,
    hits: &mut hit_registry_t,
) {
    let x0 = area.x.saturating_add(2);
    let x1 = area.right().saturating_sub(2);
    let rect = Rect::new(area.x, y, area.width, 1);
    fill(buf, rect, Style::new().bg(row.bg));
    let st = Style::new().bg(row.bg);
    if let Some(rail) = row.rail {
        put_str(buf, area.x, y, area.right(), "▌", st.fg(rail));
    }
    let guide = mix(row.bg, theme.fg_faint, 0.7);
    for k in 0..depth {
        put_str(
            buf,
            x0.saturating_add((k * 2) as u16),
            y,
            x1,
            "▏",
            st.fg(guide),
        );
    }
    let x = x0.saturating_add((depth * 2) as u16);
    put_line(buf, x, y, x1, &row.line, st);
    for (col, width, action) in &row.buttons {
        let r = Rect::new(x.saturating_add(*col), y, *width, 1).intersection(rect);
        hits.add(r, action.clone());
    }
}

/// The chips a row offers on hover or selection, as (glyph, action).
fn row_chips(index: usize, item: &sidebar_item_t) -> Vec<(&'static str, Action)> {
    let check = (
        if item.selected { CheckOn } else { CheckOff },
        Action::Select(SelectAction::ToggleAt(index)),
    );
    match &item.kind {
        SidebarItemKind::Folder { .. } => {
            let mut chips = Vec::with_capacity(5);
            chips.push(check);
            // A top-level folder is level 1, so a folder at depth d is level d + 1.
            if item.depth + 1 < harness::organize::MaxFolderDepth {
                chips.push(("+", Action::NewSubfolderAt(index)));
            }
            chips.push(("✎", Action::RenameFolderAt(index)));
            chips.push(("⇢", Action::PickUpAt(index)));
            chips.push(("✕", Action::DeleteFolderAt(index)));
            chips
        }
        SidebarItemKind::Session { .. } => vec![check, ("⇢", Action::PickUpAt(index))],
        _ => Vec::new(),
    }
}

/// Draw one row of the tree: a folder or a conversation.
fn draw_tree_row(
    buf: &mut Buffer,
    ctx: &tree_ctx_t<'_, '_>,
    y: u16,
    index: usize,
    item: &sidebar_item_t,
    hits: &mut hit_registry_t,
) {
    let (area, bg, input, moving, theme) = (ctx.area, ctx.bg, ctx.input, ctx.moving, ctx.theme);
    let x0 = area.x.saturating_add(2);
    let x1 = area.right().saturating_sub(2);
    let rect = Rect::new(area.x, y, area.width, 1);
    let selected = input.focused && index == input.selected;
    let hovered = input
        .hover
        .is_some_and(|(hx, hy)| rect.contains(Position::new(hx, hy)));
    let hot = input.hot == Some(index);
    let refused = matches!(item.mark, MoveMark::Refused { .. });
    let row_bg = if hot && refused {
        refused_bg(bg, theme)
    } else if hot {
        hot_bg(bg, theme)
    } else if item.mark == MoveMark::Source {
        theme.inset
    } else if item.selected && (selected || hovered) && !moving {
        mix(theme.marked, theme.fg, 0.06)
    } else if item.selected {
        theme.marked
    } else if item.active {
        theme.select
    } else if (selected || hovered) && !moving {
        theme.hover
    } else {
        bg
    };
    fill(buf, rect, Style::new().bg(row_bg));
    let st = Style::new().bg(row_bg);
    if hot {
        let rail = if refused { theme.error } else { theme.ice };
        put_str(buf, area.x, y, area.right(), "▌", st.fg(rail));
    } else if item.active {
        put_str(buf, area.x, y, area.right(), "▌", st.fg(theme.agent));
    } else if item.selected {
        put_str(buf, area.x, y, area.right(), "▎", st.fg(theme.ice));
    }
    if selected && !moving {
        put_str(
            buf,
            area.x.saturating_add(1),
            y,
            area.right(),
            "›",
            st.fg(theme.ice),
        );
    }
    let guide = mix(row_bg, theme.fg_faint, 0.7);
    for k in 0..item.depth {
        put_str(
            buf,
            x0.saturating_add((k * 2) as u16),
            y,
            x1,
            "▏",
            st.fg(guide),
        );
    }
    hits.add_target(rect, Target::Row(index));

    // Right side: the chips, a move hint, or the detail.
    let show_chips = input.chips && !moving && (hovered || selected);
    let mut right_start = x1;
    if show_chips {
        let chips = row_chips(index, item);
        let mut cx = x1;
        let mut placed = Vec::with_capacity(chips.len());
        for (glyph, action) in chips.iter().rev() {
            cx = cx.saturating_sub(3);
            placed.push((cx, *glyph, action.clone()));
        }
        let floor = x0.saturating_add((item.depth * 2 + 6) as u16);
        if cx >= floor {
            for (cx, glyph, action) in placed {
                let chip_rect = Rect::new(cx, y, 3, 1);
                let over = input
                    .hover
                    .is_some_and(|(hx, hy)| chip_rect.contains(Position::new(hx, hy)));
                let (fg, chip_bg) = match (over, glyph) {
                    (true, "✕") => (theme.error, theme.select),
                    (true, _) => (theme.fg, theme.select),
                    (false, CheckOn) => (theme.ice, theme.inset),
                    (false, _) => (theme.fg_dim, theme.inset),
                };
                put_str(
                    buf,
                    cx,
                    y,
                    x1,
                    &format!(" {glyph} "),
                    Style::new().fg(fg).bg(chip_bg),
                );
                hits.add(chip_rect, action);
            }
            right_start = cx;
        }
    }
    let pulse = if input.animate {
        (input.clock as f32 / 300.0).sin() * 0.5 + 0.5
    } else {
        1.0
    };
    let text_x = match item.kind {
        SidebarItemKind::Folder { .. } => x0.saturating_add((item.depth * 2) as u16),
        _ => x0.saturating_add((item.depth * 2 + 2) as u16),
    };
    match &item.kind {
        SidebarItemKind::Folder { collapsed, .. } => {
            let (arrow, arrow_fg) = if item.mark == MoveMark::Source {
                ("⇢ ", theme.ice)
            } else if *collapsed {
                ("▸ ", theme.fg_faint)
            } else {
                ("▾ ", theme.fg_faint)
            };
            let nx = put_str(buf, text_x, y, x1, arrow, st.fg(arrow_fg));
            let name_fg = match &item.mark {
                MoveMark::Refused { .. } if hot => theme.error,
                MoveMark::Refused { .. } | MoveMark::Source => theme.fg_faint,
                _ if hot || selected || hovered => theme.fg,
                _ => theme.fg_dim,
            };
            let right = if show_chips && right_start < x1 {
                None
            } else {
                Some(folder_right(item, hot, collapsed, theme, pulse))
            };
            let right_w = right.as_ref().map(|r| r.width() as u16).unwrap_or(0);
            let end = right_start.min(x1.saturating_sub(right_w + u16::from(right_w > 0)));
            let name = truncate(&item.title, end.saturating_sub(nx) as usize);
            put_str(
                buf,
                nx,
                y,
                end,
                &name,
                st.fg(name_fg).add_modifier(Modifier::BOLD),
            );
            if let Some(r) = right {
                put_line(buf, x1.saturating_sub(right_w).max(nx), y, x1, &r, st);
            }
        }
        SidebarItemKind::Session { .. } => {
            if item.mark == MoveMark::Source {
                put_str(buf, text_x.saturating_sub(2), y, x1, "⇢", st.fg(theme.ice));
            } else if let Some(color) = item.color {
                put_str(
                    buf,
                    text_x.saturating_sub(2),
                    y,
                    x1,
                    TagDot,
                    st.fg(tag_rgb(color)),
                );
            }
            let detail_w = if show_chips && right_start < x1 {
                0
            } else {
                str_width(&item.detail) as u16
            };
            let tag_w: u16 = if item.demo && detail_w > 0 { 5 } else { 0 };
            let dot_w: u16 = if item.activity == Activity::Idle {
                0
            } else {
                2
            };
            let title_end = right_start
                .min(x1)
                .saturating_sub(detail_w + tag_w + dot_w + 1);
            let title_style = match item.mark {
                MoveMark::Inert | MoveMark::Source => st.fg(theme.fg_faint),
                _ if item.archived => st.fg(theme.fg_faint),
                _ if item.active => st.fg(theme.fg).add_modifier(Modifier::BOLD),
                _ if item.selected => st.fg(theme.fg),
                _ => st.fg(theme.fg_dim),
            };
            let mut tx = text_x;
            if item.pinned {
                tx = put_str(
                    buf,
                    tx,
                    y,
                    title_end,
                    PinGlyph,
                    st.fg(mix(row_bg, theme.question, 0.85)),
                );
                tx = tx.saturating_add(1);
            }
            let title = truncate(&item.title, title_end.saturating_sub(tx) as usize);
            put_str(buf, tx, y, title_end, &title, title_style);
            let mut rx = right_start.min(x1);
            if detail_w > 0 {
                rx = put_str_right(buf, rx, y, x0, &item.detail, st.fg(theme.fg_faint));
                if item.demo {
                    rx = put_str_right(
                        buf,
                        rx,
                        y,
                        x0,
                        "demo ",
                        st.fg(mix(theme.side, theme.approval, 0.6)),
                    );
                }
            }
            if let Some((dot, color)) = activity_dot(item.activity, theme, pulse) {
                put_str_right(buf, rx, y, x0, dot, st.fg(color));
            }
        }
        SidebarItemKind::NewChat | SidebarItemKind::Root => {}
    }
}

/// The right side of a folder row: the reason a move is refused, a drop hint, or the count of
/// conversations inside (and a dot when a folded folder holds a working conversation).
fn folder_right(
    item: &sidebar_item_t,
    hot: bool,
    collapsed: &bool,
    theme: &theme_t,
    pulse: f32,
) -> line_t {
    match &item.mark {
        MoveMark::Refused { reason } if hot => {
            line_t::plain(reason.clone(), Style::new().fg(theme.error))
        }
        MoveMark::Allowed if hot => line_t::plain("move here", Style::new().fg(theme.ice)),
        _ => {
            let mut line = line_t::blank();
            if *collapsed && let Some((dot, color)) = activity_dot(item.activity, theme, pulse) {
                line.push(dot, Style::new().fg(color));
            }
            line.push(item.detail.clone(), Style::new().fg(theme.fg_faint));
            line
        }
    }
}

/// The dot of a working conversation: pulsing ice while it generates, amber while it waits.
fn activity_dot(activity: Activity, theme: &theme_t, pulse: f32) -> Option<(&'static str, Color)> {
    match activity {
        Activity::Idle => None,
        Activity::Generating => Some(("● ", mix(theme.ice_deep, theme.ice, pulse))),
        Activity::Waiting => Some(("● ", theme.approval)),
        Activity::Tool => Some(("● ", theme.tool)),
    }
}
