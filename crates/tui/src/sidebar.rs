/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The right-hand column: the user's way around the UI. A new-chat button, the state of the
//! current turn and call, the model, and the conversations grouped by project. Sections are a
//! bold label over dim values, separated by space, never by lines.

use harness::api::{SessionPhase, runtime_info_t};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::banner::{chip, short_path};
use crate::canvas::{fill, put_line, put_str, put_str_right};
use crate::input::{Action, hit_registry_t};
use crate::strip::strip_line;
use crate::text::{format_ms, group_digits, line_t, sanitize, truncate};
use crate::theme::{mix, theme_t};
use crate::view_model::transcript_t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Generating,
    Waiting,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarItemKind {
    NewChat,
    Project { name: String, collapsed: bool },
    Session { id: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct sidebar_item_t {
    pub kind: SidebarItemKind,
    pub title: String,
    pub detail: String,
    pub active: bool,
    pub activity: Activity,
    pub demo: bool,
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
    out.push((0, rows));

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
    out.push((0, rows));

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
    out.push((1, rows));

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
    out.push((2, rows));
    out
}

/// Draw the sidebar into `area`. Returns the conversation scroll offset it used.
pub fn draw_sidebar(
    buf: &mut Buffer,
    area: Rect,
    input: &sidebar_input_t<'_>,
    theme: &theme_t,
    hits: &mut hit_registry_t,
) -> usize {
    if area.width < 8 || area.height < 4 {
        return input.scroll;
    }
    let bg = if input.drawer { theme.dock } else { theme.side };
    fill(buf, area, Style::new().bg(bg).fg(theme.fg));
    let x0 = area.x.saturating_add(2);
    let x1 = area.right().saturating_sub(2);
    let inner = x1.saturating_sub(x0) as usize;
    let base = Style::new().bg(bg);
    let mut y = area.y.saturating_add(1);
    let bottom = area.bottom();

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
    y = y.saturating_add(2);

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
    y = y.saturating_add(2);

    // Footer: cwd and version, at the very bottom.
    let footer_h = 3u16;
    let footer_y = bottom.saturating_sub(footer_h);

    // Fit sections above the conversations; drop low-priority ones when short.
    let mut secs = sections(input, inner, theme);
    let min_list = 6usize;
    loop {
        let need: usize = secs.iter().map(|(_, r)| r.len() + 1).sum();
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
        y = y.saturating_add(1);
    }

    // Conversations.
    let mut scroll = input.scroll;
    if y < footer_y {
        put_line(buf, x0, y, x1, &label("conversations", theme), base);
        y = y.saturating_add(1);
        let list: Vec<(usize, &sidebar_item_t)> = input
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| !matches!(i.kind, SidebarItemKind::NewChat))
            .collect();
        let rows = footer_y.saturating_sub(y).saturating_sub(1) as usize;
        if rows > 0 {
            if let Some(pos) = list.iter().position(|(i, _)| *i == input.selected) {
                if pos < scroll {
                    scroll = pos;
                } else if pos >= scroll + rows {
                    scroll = pos + 1 - rows;
                }
            }
            scroll = scroll.min(list.len().saturating_sub(rows));
            for (index, item) in list.iter().skip(scroll).take(rows) {
                let rect = Rect::new(area.x, y, area.width, 1);
                let selected = input.focused && *index == input.selected;
                let hovered = input
                    .hover
                    .is_some_and(|(hx, hy)| rect.contains(Position::new(hx, hy)));
                let row_bg = if item.active {
                    theme.select
                } else if selected || hovered {
                    theme.hover
                } else {
                    bg
                };
                fill(buf, rect, Style::new().bg(row_bg));
                let st = Style::new().bg(row_bg);
                if item.active {
                    put_str(buf, area.x, y, area.right(), "▌", st.fg(theme.agent));
                }
                if selected {
                    put_str(
                        buf,
                        area.x.saturating_add(1),
                        y,
                        area.right(),
                        "›",
                        st.fg(theme.ice),
                    );
                }
                let pulse = if input.animate {
                    (input.clock as f32 / 300.0).sin() * 0.5 + 0.5
                } else {
                    1.0
                };
                let text_row = Rect::new(x0, y, x1.saturating_sub(x0), 1);
                draw_item(buf, text_row, item, st, theme, pulse);
                hits.add(rect, Action::SidebarActivateAt(*index));
                y = y.saturating_add(1);
            }
            if list.len() > scroll + rows {
                put_str(
                    buf,
                    x0,
                    y,
                    x1,
                    &format!("↓ {} more", list.len() - scroll - rows),
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
    let last_w = crate::text::str_width(&last).min(inner);
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
    put_str(
        buf,
        x0,
        fy.saturating_add(1),
        x1,
        &version,
        base.fg(theme.fg_faint),
    );
    scroll
}

/// Draw one conversation-list row into `row` (the padded text columns). `pulse` is the
/// brightness of a generating session's dot.
fn draw_item(
    buf: &mut Buffer,
    row: Rect,
    item: &sidebar_item_t,
    st: Style,
    theme: &theme_t,
    pulse: f32,
) {
    let (x0, y, x1) = (row.x, row.y, row.right());
    match &item.kind {
        SidebarItemKind::NewChat => {}
        SidebarItemKind::Project { collapsed, .. } => {
            let arrow = if *collapsed { "▸ " } else { "▾ " };
            let nx = put_str(buf, x0, y, x1, arrow, st.fg(theme.fg_faint));
            put_str(
                buf,
                nx,
                y,
                x1.saturating_sub(4),
                &item.title,
                st.fg(theme.fg_dim).add_modifier(Modifier::BOLD),
            );
            put_str_right(buf, x1, y, x0, &item.detail, st.fg(theme.fg_faint));
        }
        SidebarItemKind::Session { .. } => {
            let detail_w = crate::text::str_width(&item.detail) as u16;
            let tag_w: u16 = if item.demo { 5 } else { 0 };
            let dot_w: u16 = if item.activity == Activity::Idle {
                0
            } else {
                2
            };
            let title_end = x1.saturating_sub(detail_w + tag_w + dot_w + 1);
            let x = x0.saturating_add(2);
            let title_style = if item.active {
                st.fg(theme.fg).add_modifier(Modifier::BOLD)
            } else {
                st.fg(theme.fg_dim)
            };
            let title = truncate(&item.title, title_end.saturating_sub(x) as usize);
            put_str(buf, x, y, title_end, &title, title_style);
            let mut rx = x1;
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
            match item.activity {
                Activity::Idle => {}
                Activity::Generating => {
                    put_str_right(
                        buf,
                        rx,
                        y,
                        x0,
                        "● ",
                        st.fg(mix(theme.ice_deep, theme.ice, pulse)),
                    );
                }
                Activity::Waiting => {
                    put_str_right(buf, rx, y, x0, "● ", st.fg(theme.approval));
                }
            }
        }
    }
}
