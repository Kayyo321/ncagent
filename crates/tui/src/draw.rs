/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Drawing a whole frame from the app state, and rebuilding the hit registry as it goes.

use harness::api::SessionPhase;
use harness::approval::{ApprovalDecision, ApprovalMode};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};

use crate::app::{ToastKind, app_t};
use crate::banner::chip;
use crate::canvas::{fill, put_line, put_str, put_str_right, tint};
use crate::help::help_lines;
use crate::input::{Action, Focus, Modal, Target};
use crate::layout::{self, SidebarMode, layout_input_t};
use crate::sidebar::{draw_sidebar, sidebar_input_t};
use crate::text::{line_t, sanitize, span_t, str_width, truncate, wrap_spans};
use crate::theme::{mix, quantize_buffer, theme_t};
use crate::transcript::render_ctx_t;

/// Rows of message text the composer grows to before it scrolls.
const MaxComposerRows: usize = 8;

/// Columns inside a docked panel: margin, rail, space on the left, one on the right.
fn dock_inner(width: u16) -> usize {
    usize::from(width.saturating_sub(4)).max(1)
}

struct widget_t {
    lines: Vec<line_t>,
    /// Clickable spans: (row, start column, width, action).
    buttons: Vec<(usize, u16, u16, Action)>,
    rail: Color,
}

fn approval_widget(app: &app_t, width: usize) -> Option<widget_t> {
    let theme = &app.theme;
    let s = app.active()?;
    let (_, tool) = s.transcript.pending_approval()?;
    let req = tool.approval.as_ref()?;
    let mut lines = Vec::new();
    let mut head = line_t::new(vec![span_t::new(
        "◇ approval needed",
        Style::new().fg(theme.approval).add_modifier(Modifier::BOLD),
    )]);
    let right = format!(
        "{} · {}",
        tool.class.name(),
        truncate(&sanitize(&tool.tool, false), 24)
    );
    let pad = width.saturating_sub(head.width() + str_width(&right));
    if pad > 1 {
        head.push(" ".repeat(pad), Style::new());
        head.push(right, Style::new().fg(theme.fg_faint));
    }
    lines.push(head);
    lines.extend(wrap_spans(
        &[
            span_t::new("ncagent wants to run ", Style::new().fg(theme.fg)),
            span_t::new(
                sanitize(&tool.tool, false),
                Style::new().fg(theme.tool).add_modifier(Modifier::BOLD),
            ),
        ],
        width,
        &[],
        &[],
    ));
    lines.extend(wrap_spans(
        &[span_t::new(
            format!("asked because {}", sanitize(&req.reason, false)),
            Style::new().fg(theme.fg_dim),
        )],
        width,
        &[],
        &[],
    ));
    let summary = crate::text::hard_wrap(
        &sanitize(&req.summary, true),
        width.saturating_sub(2).max(1),
    );
    let total = summary.len();
    for row in summary.iter().take(6) {
        let pad = width.saturating_sub(2).saturating_sub(str_width(row));
        lines.push(line_t::plain(
            format!(" {row}{} ", " ".repeat(pad)),
            Style::new().fg(theme.fg).bg(theme.inset),
        ));
    }
    if total > 6 {
        lines.push(line_t::plain(
            format!(
                "+{} more line(s) · the full text is in the tool block above",
                total - 6
            ),
            Style::new().fg(theme.fg_faint),
        ));
    }
    lines.push(line_t::blank());
    let labels = [
        ("y", "allow once", ApprovalDecision::Once),
        ("a", "allow for session", ApprovalDecision::Session),
        ("n", "deny", ApprovalDecision::Deny),
    ];
    let mut buttons = Vec::new();
    let row = lines.len();
    let mut line = line_t::blank();
    let mut col = 0u16;
    for (i, (key, label, decision)) in labels.iter().enumerate() {
        let selected = app.approval_sel == i;
        let (fg, bg) = match (selected, decision) {
            (true, ApprovalDecision::Deny) => (theme.bg, theme.error),
            (true, _) => (theme.bg, theme.approval),
            (false, _) => (theme.fg, theme.inset),
        };
        let text_key = format!(" {key} ");
        let text_label = format!("{label} ");
        let w = (str_width(&text_key) + str_width(&text_label)) as u16;
        line.push(
            text_key,
            Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
        );
        line.push(text_label, Style::new().fg(fg).bg(bg));
        buttons.push((row, col, w, Action::Approve(*decision)));
        col = col.saturating_add(w);
        line.push("  ", Style::new());
        col = col.saturating_add(2);
    }
    let hint = "←→ choose · enter · esc stops the turn";
    if usize::from(col) + str_width(hint) < width {
        line.push(hint, Style::new().fg(theme.fg_faint));
    }
    lines.push(line);
    Some(widget_t {
        lines,
        buttons,
        rail: theme.approval,
    })
}

fn question_widget(app: &app_t, width: usize) -> Option<widget_t> {
    let theme = &app.theme;
    let s = app.active()?;
    let (_, q) = s.transcript.pending_question()?;
    let mut lines = Vec::new();
    let mut buttons = Vec::new();
    let mut head = line_t::new(vec![span_t::new(
        "? ncagent asks",
        Style::new().fg(theme.question).add_modifier(Modifier::BOLD),
    )]);
    let right = "1-9 pick · ↑↓ choose · enter send";
    let pad = width.saturating_sub(head.width() + str_width(right));
    if pad > 1 {
        head.push(" ".repeat(pad), Style::new());
        head.push(right, Style::new().fg(theme.fg_faint));
    }
    lines.push(head);
    let question = crate::text::wrap_plain(
        &sanitize(&q.question, true),
        width,
        Style::new().fg(theme.fg),
    );
    lines.extend(question.into_iter().take(4));
    for (i, option) in q.options.iter().enumerate().take(9) {
        let selected = app.question_sel == i;
        let bg = if selected { theme.select } else { theme.inset };
        let text = truncate(&sanitize(option, false), width.saturating_sub(6));
        let used = 5 + str_width(&text);
        let mut line = line_t::new(vec![
            span_t::new(
                format!(" {} ", i + 1),
                Style::new()
                    .fg(theme.question)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            span_t::new(format!(" {text}"), Style::new().fg(theme.fg).bg(bg)),
        ]);
        line.push(
            " ".repeat(width.saturating_sub(used).min(4)),
            Style::new().bg(bg),
        );
        let w = line.width() as u16;
        buttons.push((lines.len(), 0, w, Action::PickOption(i)));
        lines.push(line);
    }
    let free_selected = app.question_sel >= q.options.len();
    let bg = if free_selected {
        theme.select
    } else {
        theme.inset
    };
    let mut free = line_t::new(vec![span_t::new(
        " › ",
        Style::new().fg(theme.question).bg(bg),
    )]);
    if app.answer.is_empty() {
        free.push(
            "type your own answer",
            Style::new().fg(theme.fg_faint).bg(bg),
        );
    } else {
        let text = sanitize(app.answer.text(), false);
        let w = width.saturating_sub(4);
        let shown = if str_width(&text) > w {
            let chars: Vec<char> = text.chars().collect();
            let mut tail = String::new();
            let mut used = 0usize;
            for c in chars.iter().rev() {
                let cw = str_width(&c.to_string());
                if used + cw > w.saturating_sub(1) {
                    break;
                }
                tail.insert(0, *c);
                used += cw;
            }
            format!("…{tail}")
        } else {
            text
        };
        free.push(shown, Style::new().fg(theme.fg).bg(bg));
        free.push("▌", Style::new().fg(theme.question).bg(bg));
    }
    let used = free.width();
    free.push(" ".repeat(width.saturating_sub(used)), Style::new().bg(bg));
    lines.push(free);
    Some(widget_t {
        lines,
        buttons,
        rail: theme.question,
    })
}

/// Draw the whole frame.
pub fn draw(app: &mut app_t, frame: &mut Frame<'_>) {
    let area = frame.area();
    app.hits.clear();
    let theme = app.theme.clone();
    let buf = frame.buffer_mut();
    fill(buf, area, Style::new().bg(theme.bg).fg(theme.fg));
    if app.size != (area.width, area.height) {
        app.resize(area.width, area.height);
    }
    let modal = app.modal();
    let popup = app.popup();

    // First pass to know the main width, then size the dock to its content.
    let mut input = layout_input_t {
        width: area.width,
        height: area.height,
        sidebar_pinned: app.sidebar_pinned,
        drawer_open: app.drawer_open,
        dock_rows: 3,
        queue_rows: u16::from(app.active().is_some_and(|s| !s.queued.is_empty())),
        popup_rows: popup.len().min(9) as u16,
        help_open: app.help_open,
    };
    let first = layout::compute(input);
    if first.too_small {
        app.layout = first;
        draw_too_small(buf, area, &theme);
        return;
    }
    let inner = dock_inner(first.main.width);
    let widget = match modal {
        Modal::Approval => approval_widget(app, inner),
        Modal::Question => question_widget(app, inner),
        _ => None,
    };
    let composer_rows = app.composer.layout(inner).0.len().clamp(1, MaxComposerRows);
    let pad_top = u16::from(area.height >= 20);
    input.dock_rows = match &widget {
        Some(w) => (w.lines.len() as u16).saturating_add(pad_top * 2),
        None => (composer_rows as u16).saturating_add(1 + pad_top * 2),
    };
    let lay = layout::compute(input);
    app.layout = lay;

    draw_transcript(app, buf, lay.transcript, lay.scrollbar);
    draw_queue(app, buf, lay.queue);
    match &widget {
        Some(w) => draw_widget(app, buf, lay.dock, w, pad_top),
        None => draw_composer(app, buf, lay.dock, pad_top, composer_rows),
    }
    if !popup.is_empty() {
        draw_popup(app, buf, lay.popup, &popup);
    }
    draw_status(app, buf, lay.status);
    if lay.sidebar_mode() != SidebarMode::Hidden {
        draw_side(
            app,
            buf,
            lay.sidebar,
            lay.sidebar_mode() == SidebarMode::Drawer,
        );
    }
    draw_toast(app, buf, lay.transcript);
    if app.help_open {
        draw_help(app, buf, lay.help);
    }
    quantize_buffer(buf, app.options.color);
}

fn draw_too_small(buf: &mut Buffer, area: Rect, theme: &theme_t) {
    let text = format!(
        "terminal too small ({}x{}, need at least {}x{})",
        area.width,
        area.height,
        layout::MinWidth,
        layout::MinHeight
    );
    let lines = crate::text::wrap_plain(
        &text,
        usize::from(area.width.max(1)),
        Style::new().fg(theme.fg_dim),
    );
    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    for (i, l) in lines.iter().enumerate() {
        let w = l.width() as u16;
        let x = area.width.saturating_sub(w) / 2;
        put_line(
            buf,
            x,
            top.saturating_add(i as u16),
            area.right(),
            l,
            Style::new().bg(theme.bg),
        );
    }
}

fn draw_transcript(app: &mut app_t, buf: &mut Buffer, area: Rect, scrollbar: Rect) {
    let theme = app.theme.clone();
    let content_x = area.x.saturating_add(3);
    let content_right = area.right().saturating_sub(2);
    let width = usize::from(content_right.saturating_sub(content_x)).max(1);
    let focus = app.focus;
    let hover = app.hover;
    let clock = app.clock;
    let animate = app.animate;
    let info = &app.info;
    let Some(live) = app.sessions.get_mut(app.active) else {
        return;
    };
    let ctx = render_ctx_t {
        theme: &theme,
        width,
        clock,
        animate,
        info,
    };
    let previous_total = live.scroll.total;
    live.view.prepare(&live.transcript, &ctx);
    let total = live.view.total;
    let viewport = usize::from(area.height);
    live.scroll.total = total;
    live.scroll.viewport = viewport;
    if let Some((id, offset)) = live.scroll.anchor.take()
        && let Some(i) = live.transcript.index_of(id)
        && let Some(start) = live.view.starts.get(i)
    {
        live.scroll.top = start.saturating_add(offset);
    }
    if live.scroll.follow {
        live.scroll.top = live.scroll.max_top();
        live.scroll.unseen = 0;
    } else if total > previous_total && previous_total > 0 {
        live.scroll.unseen = live.scroll.unseen.saturating_add(total - previous_total);
    }
    live.scroll.top = live.scroll.top.min(live.scroll.max_top());
    let top = live.scroll.top;
    let selected = if focus == Focus::Transcript {
        live.selected_block
    } else {
        None
    };

    for r in 0..area.height {
        let g = top.saturating_add(usize::from(r));
        if g >= total {
            break;
        }
        let y = area.y.saturating_add(r);
        let Some(bi) = live.view.block_at(g) else {
            continue;
        };
        let Some(block) = live.transcript.blocks.get(bi) else {
            continue;
        };
        let start = live.view.starts.get(bi).copied().unwrap_or(0);
        let Some(rows) = live.view.rows_of(block.id) else {
            continue;
        };
        let local = g.saturating_sub(start);
        let Some(row) = rows.rows.get(local) else {
            continue;
        };
        if let Some(panel) = row.panel {
            fill(
                buf,
                Rect::new(
                    area.x.saturating_add(1),
                    y,
                    content_right
                        .saturating_add(1)
                        .saturating_sub(area.x.saturating_add(1)),
                    1,
                ),
                Style::new().bg(panel),
            );
        }
        let base = Style::new().bg(row.panel.unwrap_or(theme.bg));
        if let Some((glyph, color)) = row.rail {
            put_str(
                buf,
                area.x.saturating_add(1),
                y,
                content_x,
                glyph,
                base.fg(color),
            );
        }
        put_line(buf, content_x, y, content_right, &row.line, base);
        let row_rect = Rect::new(
            area.x,
            y,
            content_right.saturating_sub(area.x).saturating_add(1),
            1,
        );
        let is_selected = selected == Some(block.id);
        if is_selected {
            put_str(
                buf,
                area.x,
                y,
                area.x.saturating_add(1),
                if local == 0 { "›" } else { " " },
                Style::new().fg(theme.ice).bg(theme.select),
            );
        }
        let hovered = hover.is_some_and(|(hx, hy)| row_rect.contains(Position::new(hx, hy)));
        let clickable = matches!(
            row.target,
            Some(Action::ToggleBlock(_)) | Some(Action::Cancel)
        );
        if hovered && clickable && row.panel.is_none() {
            for x in content_x..content_right {
                if let Some(cell) = buf.cell_mut((x, y))
                    && cell.bg == theme.bg
                {
                    cell.set_bg(theme.side);
                }
            }
        }
        let action = row
            .target
            .clone()
            .unwrap_or(Action::SelectBlockId(block.id));
        app.hits.add(row_rect, action);
    }

    // Sticky speaker label when a block's header has scrolled off the top.
    if let Some(bi) = live.view.block_at(top)
        && let Some(block) = live.transcript.blocks.get(bi)
        && let Some(rows) = live.view.rows_of(block.id)
    {
        let start = live.view.starts.get(bi).copied().unwrap_or(0);
        let local = top.saturating_sub(start);
        if local > 0
            && local < rows.rows.len()
            && let Some(label) = &rows.sticky
        {
            let y = area.y;
            let bg = theme.inset;
            fill(
                buf,
                Rect::new(
                    area.x.saturating_add(1),
                    y,
                    content_right.saturating_sub(area.x),
                    1,
                ),
                Style::new().bg(bg),
            );
            let rail = rows
                .rows
                .first()
                .and_then(|r| r.rail)
                .map(|(_, c)| c)
                .unwrap_or(theme.fg_faint);
            put_str(
                buf,
                area.x.saturating_add(1),
                y,
                content_x,
                "▌",
                Style::new().fg(rail).bg(bg),
            );
            put_line(buf, content_x, y, content_right, label, Style::new().bg(bg));
            put_str_right(
                buf,
                content_right,
                y,
                content_x,
                "↑ continued",
                Style::new().fg(theme.fg_faint).bg(bg),
            );
        }
    }

    // Scrollbar: a tone track with a lighter thumb.
    let max_top = live.scroll.max_top();
    if total > viewport && scrollbar.height > 0 {
        let track_h = usize::from(scrollbar.height);
        let thumb = (viewport.saturating_mul(track_h) / total.max(1)).clamp(1, track_h);
        let pos = top
            .saturating_mul(track_h - thumb)
            .checked_div(max_top)
            .unwrap_or(0);
        let hovered = hover.is_some_and(|(hx, hy)| scrollbar.contains(Position::new(hx, hy)));
        let thumb_color = if hovered || app.dragging.is_some() {
            theme.fg_dim
        } else {
            theme.fg_faint
        };
        for i in 0..track_h {
            let y = scrollbar.y.saturating_add(i as u16);
            let bg = if i >= pos && i < pos + thumb {
                thumb_color
            } else {
                theme.side
            };
            fill(buf, Rect::new(scrollbar.x, y, 1, 1), Style::new().bg(bg));
        }
        app.hits.add_target(scrollbar, Target::Scrollbar);
    }

    // "Jump to latest" chip.
    if !live.scroll.follow && top < max_top {
        let text = if live.scroll.unseen > 0 {
            format!(" ↓ {} new · jump to latest ", live.scroll.unseen)
        } else {
            " ↓ jump to latest ".to_string()
        };
        let w = str_width(&text) as u16;
        let y = area.bottom().saturating_sub(1);
        let x = content_right.saturating_sub(w);
        let r = Rect::new(x, y, w, 1);
        put_str(
            buf,
            x,
            y,
            content_right,
            &text,
            Style::new()
                .fg(theme.fg)
                .bg(theme.select)
                .add_modifier(Modifier::BOLD),
        );
        app.hits.add(r, Action::JumpLatest);
    }
}

fn draw_queue(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    if area.height == 0 {
        return;
    }
    let theme = &app.theme;
    let Some(s) = app.active() else {
        return;
    };
    let x0 = area.x.saturating_add(3);
    let right = area.right().saturating_sub(2);
    let mut x = put_str(
        buf,
        x0,
        area.y,
        right,
        "queued ",
        Style::new().fg(theme.fg_faint),
    );
    let clear = " ctrl+x clear ";
    let clear_w = str_width(clear) as u16;
    let limit = right.saturating_sub(clear_w + 1);
    for (i, q) in s.queued.iter().enumerate() {
        let text = format!(" {} ", truncate(&sanitize(q, false), 24));
        let w = str_width(&text) as u16;
        if x.saturating_add(w) > limit {
            let more = format!("+{}", s.queued.len() - i);
            put_str(
                buf,
                x,
                area.y,
                limit,
                &more,
                Style::new().fg(theme.fg_faint),
            );
            break;
        }
        put_str(
            buf,
            x,
            area.y,
            limit,
            &text,
            Style::new().fg(theme.fg).bg(theme.inset),
        );
        x = x.saturating_add(w + 1);
    }
    let cx = right.saturating_sub(clear_w);
    put_str(
        buf,
        cx,
        area.y,
        right,
        clear,
        Style::new().fg(theme.fg_dim).bg(theme.panel),
    );
    app.hits
        .add(Rect::new(cx, area.y, clear_w, 1), Action::ClearQueue);
}

fn draw_composer(app: &mut app_t, buf: &mut Buffer, area: Rect, pad_top: u16, rows: usize) {
    let theme = app.theme.clone();
    fill(buf, area, Style::new().bg(theme.dock));
    let busy = app.busy();
    let rail = if busy {
        let t = if app.animate {
            (app.clock as f32 / 420.0).sin() * 0.5 + 0.5
        } else {
            1.0
        };
        mix(theme.ice_deep, theme.ice, t)
    } else {
        theme.composer_rail
    };
    for y in area.top()..area.bottom() {
        put_str(
            buf,
            area.x.saturating_add(1),
            y,
            area.right(),
            "▌",
            Style::new().fg(rail).bg(theme.dock),
        );
    }
    let x0 = area.x.saturating_add(3);
    let right = area.right().saturating_sub(1);
    let inner = usize::from(right.saturating_sub(x0)).max(1);
    let y0 = area.y.saturating_add(pad_top);
    let base = Style::new().bg(theme.dock);
    let (lines, (crow, ccol)) = app.composer.layout(inner);
    let first = crow.saturating_sub(rows.saturating_sub(1));
    if app.composer.is_empty() {
        let placeholder = if busy {
            "Message ncagent · it is queued until this turn ends"
        } else if app.simulated() {
            "Message ncagent, or try a demo word…"
        } else {
            "Message ncagent…"
        };
        put_str(
            buf,
            x0.saturating_add(1),
            y0,
            right,
            &truncate(placeholder, inner.saturating_sub(1)),
            base.fg(theme.fg_faint),
        );
    } else {
        for (i, line) in lines.iter().skip(first).take(rows).enumerate() {
            put_str(
                buf,
                x0,
                y0.saturating_add(i as u16),
                right,
                line,
                base.fg(theme.fg),
            );
        }
    }
    // A solid block cursor that does not blink.
    if app.focus == Focus::Input && app.modal() == Modal::None {
        let cy = y0.saturating_add(crow.saturating_sub(first) as u16);
        let cx = x0.saturating_add(ccol as u16);
        if cx < right
            && let Some(cell) = buf.cell_mut((cx, cy))
        {
            cell.set_style(Style::new().fg(theme.bg).bg(theme.fg));
        }
    }
    // Meta row: mode, model, demo.
    let my = y0.saturating_add(rows as u16);
    let mut meta = line_t::blank();
    match app.mode() {
        ApprovalMode::Full => meta.spans.push(chip("full", theme.bg, theme.error)),
        m => meta.push(
            m.name(),
            Style::new()
                .fg(theme.composer_rail)
                .add_modifier(Modifier::BOLD),
        ),
    }
    meta.push(" approval", Style::new().fg(theme.fg_faint));
    meta.push("  ·  ", Style::new().fg(theme.fg_faint));
    meta.push(
        truncate(&sanitize(&app.info.identity.id, false), 28),
        Style::new().fg(theme.fg_dim),
    );
    if app.simulated() {
        meta.push("  ", Style::new());
        meta.spans.push(chip("DEMO", theme.bg, theme.approval));
    }
    put_line(buf, x0, my, right, &meta, base);
    app.hits.add(area, Action::SetFocus(Focus::Input));
}

fn draw_widget(app: &mut app_t, buf: &mut Buffer, area: Rect, w: &widget_t, pad_top: u16) {
    let theme = app.theme.clone();
    fill(buf, area, Style::new().bg(theme.dock));
    for y in area.top()..area.bottom() {
        put_str(
            buf,
            area.x.saturating_add(1),
            y,
            area.right(),
            "▌",
            Style::new().fg(w.rail).bg(theme.dock),
        );
    }
    let x0 = area.x.saturating_add(3);
    let right = area.right().saturating_sub(1);
    let y0 = area.y.saturating_add(pad_top);
    let base = Style::new().bg(theme.dock);
    // Keep the buttons in view: when the dock is short, drop rows from the top of the text.
    let room = usize::from(area.bottom().saturating_sub(y0));
    let skip = w.lines.len().saturating_sub(room);
    for (i, line) in w.lines.iter().enumerate().skip(skip) {
        let y = y0.saturating_add((i - skip) as u16);
        if y >= area.bottom() {
            break;
        }
        put_line(buf, x0, y, right, line, base);
    }
    for (row, col, width, action) in &w.buttons {
        if *row < skip {
            continue;
        }
        let y = y0.saturating_add((row - skip) as u16);
        if y >= area.bottom() {
            continue;
        }
        let rect = Rect::new(x0.saturating_add(*col), y, *width, 1).intersection(area);
        if app
            .hover
            .is_some_and(|(hx, hy)| rect.contains(Position::new(hx, hy)))
        {
            for x in rect.left()..rect.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.modifier.insert(Modifier::UNDERLINED);
                }
            }
        }
        app.hits.add(rect, action.clone());
    }
}

fn draw_popup(app: &mut app_t, buf: &mut Buffer, area: Rect, list: &[crate::commands::command_t]) {
    let theme = &app.theme;
    if area.height == 0 {
        return;
    }
    let selected = app.composer.popup_index.min(list.len().saturating_sub(1));
    for (i, c) in list.iter().enumerate().take(usize::from(area.height)) {
        let y = area.y.saturating_add(i as u16);
        let bg = if i == selected {
            theme.select
        } else {
            theme.inset
        };
        let row = Rect::new(area.x, y, area.width, 1);
        fill(buf, row, Style::new().bg(bg));
        let mut x = put_str(
            buf,
            area.x.saturating_add(1),
            y,
            area.right(),
            c.name,
            Style::new()
                .fg(theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        );
        if !c.args.is_empty() {
            x = put_str(
                buf,
                x.saturating_add(1),
                y,
                area.right(),
                c.args,
                Style::new().fg(theme.composer_rail).bg(bg),
            );
        }
        let about_x = area.x.saturating_add(24).max(x.saturating_add(2));
        put_str(
            buf,
            about_x,
            y,
            area.right().saturating_sub(1),
            c.about,
            Style::new().fg(theme.fg_dim).bg(bg),
        );
        app.hits.add(row, Action::RunCommand(c.name));
    }
}

fn draw_status(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    let theme = app.theme.clone();
    fill(buf, area, Style::new().bg(theme.bg));
    let x0 = area.x.saturating_add(1);
    let right = area.right().saturating_sub(1);
    let phase = app.active().map(|s| s.phase).unwrap_or(SessionPhase::Idle);
    let pulse = if app.animate {
        (app.clock as f32 / 300.0).sin() * 0.5 + 0.5
    } else {
        1.0
    };
    let (dot, who, color) = match phase {
        SessionPhase::Idle => ("●", "your turn", theme.success),
        SessionPhase::AwaitingApproval | SessionPhase::AwaitingAnswer => {
            ("●", "your call", theme.approval)
        }
        _ => ("◉", "ncagent's turn", mix(theme.ice_deep, theme.ice, pulse)),
    };
    let mut x = put_str(buf, x0, area.y, right, dot, Style::new().fg(color));
    x = put_str(
        buf,
        x.saturating_add(1),
        area.y,
        right,
        who,
        Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    if phase != SessionPhase::Idle {
        let text = format!(" · {}", app.phase_text());
        x = put_str(buf, x, area.y, right, &text, Style::new().fg(theme.fg_dim));
    }
    let left_end = x.saturating_add(2);

    let modal = app.modal();
    let busy = app.busy();
    let mut hints: Vec<(&str, &str, Option<Action>)> = Vec::new();
    match (modal, app.focus) {
        (Modal::Approval, _) => {
            hints.push(("y", "once", Some(Action::Approve(ApprovalDecision::Once))));
            hints.push((
                "a",
                "session",
                Some(Action::Approve(ApprovalDecision::Session)),
            ));
            hints.push(("n", "deny", Some(Action::Approve(ApprovalDecision::Deny))));
        }
        (Modal::Question, _) => {
            hints.push(("1-9", "pick", None));
            hints.push(("enter", "send", Some(Action::QuestionSubmit)));
        }
        (Modal::Help, _) => hints.push(("any key", "close", Some(Action::ToggleHelp))),
        (Modal::None, Focus::Transcript) => {
            hints.push(("j/k", "select", None));
            hints.push(("enter", "expand", Some(Action::ToggleSelected)));
            hints.push(("i", "input", Some(Action::SetFocus(Focus::Input))));
        }
        (Modal::None, Focus::Sidebar) => {
            hints.push(("j/k", "move", None));
            hints.push(("enter", "open", Some(Action::SidebarActivate)));
            hints.push(("n", "new", Some(Action::NewChat)));
        }
        (Modal::None, Focus::Input) => {
            if busy {
                hints.push(("enter", "queue", None));
            } else {
                hints.push(("enter", "send", Some(Action::Submit)));
                hints.push(("ctrl+j", "newline", None));
            }
        }
    }
    if busy {
        hints.push(("esc", "stop", Some(Action::Cancel)));
    }
    hints.push(("tab", "focus", Some(Action::FocusNext)));
    hints.push(("ctrl+b", "sidebar", Some(Action::ToggleSidebar)));
    hints.push(("?", "help", Some(Action::ToggleHelp)));

    // Fit from the right; drop the leftmost hints that do not fit.
    let widths: Vec<u16> = hints
        .iter()
        .map(|(k, l, _)| (str_width(k) + 1 + str_width(l)) as u16)
        .collect();
    let mut keep = hints.len();
    loop {
        let total: u16 = widths
            .iter()
            .rev()
            .take(keep)
            .fold(0u16, |a, w| a.saturating_add(*w).saturating_add(2));
        if keep == 0 || right.saturating_sub(total) >= left_end {
            break;
        }
        keep -= 1;
    }
    let shown: Vec<_> = hints
        .iter()
        .zip(widths.iter())
        .skip(hints.len() - keep)
        .collect();
    let total: u16 = shown
        .iter()
        .fold(0u16, |a, (_, w)| a.saturating_add(**w).saturating_add(2));
    let mut hx = right.saturating_sub(total).saturating_add(2);
    for ((key, label, action), w) in shown {
        let rect = Rect::new(hx, area.y, *w, 1);
        let hovered = app
            .hover
            .is_some_and(|(px, py)| rect.contains(Position::new(px, py)));
        let key_style = Style::new().fg(theme.fg).add_modifier(Modifier::BOLD);
        let label_style = if hovered && action.is_some() {
            Style::new().fg(theme.fg)
        } else {
            Style::new().fg(theme.fg_faint)
        };
        let nx = put_str(buf, hx, area.y, right, key, key_style);
        put_str(buf, nx.saturating_add(1), area.y, right, label, label_style);
        if let Some(a) = action {
            app.hits.add(rect, a.clone());
        }
        hx = hx.saturating_add(*w).saturating_add(2);
    }
}

fn draw_side(app: &mut app_t, buf: &mut Buffer, area: Rect, drawer: bool) {
    let phase_text = app.phase_text();
    let theme = app.theme.clone();
    let Some(live) = app.sessions.get(app.active) else {
        return;
    };
    let input = sidebar_input_t {
        info: &app.info,
        transcript: &live.transcript,
        phase: live.phase,
        phase_text: &phase_text,
        items: &app.sidebar_items,
        selected: app.sidebar_sel,
        focused: app.focus == Focus::Sidebar,
        scroll: app.sidebar_scroll,
        clock: app.clock,
        animate: app.animate,
        hover: app.hover,
        drawer,
    };
    let scroll = draw_sidebar(buf, area, &input, &theme, &mut app.hits);
    app.sidebar_scroll = scroll;
    if drawer {
        // A tone edge where the drawer meets the transcript.
        tint(buf, Rect::new(area.x, area.y, 1, area.height), theme.inset);
    }
}

fn draw_toast(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    let theme = &app.theme;
    let Some(t) = &app.toast else {
        return;
    };
    let color = match t.kind {
        ToastKind::Info => theme.agent,
        ToastKind::Warning => theme.approval,
        ToastKind::Error => theme.error,
    };
    let max = usize::from(area.width.saturating_sub(8)).max(8);
    let text = format!(" {} ", truncate(&t.text, max));
    let w = str_width(&text) as u16 + 1;
    let x = area.right().saturating_sub(w + 2);
    let y = area.y;
    put_str(
        buf,
        x,
        y,
        area.right(),
        "▌",
        Style::new().fg(color).bg(theme.inset),
    );
    put_str(
        buf,
        x.saturating_add(1),
        y,
        area.right(),
        &text,
        Style::new().fg(theme.fg).bg(theme.inset),
    );
}

fn draw_help(app: &mut app_t, buf: &mut Buffer, area: Rect) {
    if area.width < 10 || area.height < 4 {
        return;
    }
    let theme = app.theme.clone();
    fill(buf, area, Style::new().bg(theme.dock));
    for y in area.top()..area.bottom() {
        put_str(
            buf,
            area.x,
            y,
            area.right(),
            "▌",
            Style::new().fg(theme.ice).bg(theme.dock),
        );
    }
    let x0 = area.x.saturating_add(2);
    let right = area.right().saturating_sub(1);
    let base = Style::new().bg(theme.dock);
    put_str(
        buf,
        x0,
        area.y.saturating_add(1),
        right,
        "ncagent help",
        base.fg(theme.fg).add_modifier(Modifier::BOLD),
    );
    let lines = help_lines(&app.info, app.options.mouse, &theme);
    let rows = usize::from(area.height.saturating_sub(3));
    let max_scroll = lines.len().saturating_sub(rows);
    app.help_scroll = app.help_scroll.min(max_scroll);
    for (i, l) in lines.iter().skip(app.help_scroll).take(rows).enumerate() {
        put_line(buf, x0, area.y.saturating_add(3 + i as u16), right, l, base);
    }
    if max_scroll > 0 {
        put_str_right(
            buf,
            right,
            area.y.saturating_add(1),
            x0,
            "↑↓ scroll",
            base.fg(theme.fg_faint),
        );
    }
    app.hits.add(area, Action::ToggleHelp);
}
