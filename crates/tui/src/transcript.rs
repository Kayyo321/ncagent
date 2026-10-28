/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Blocks to rows. Every row carries its speaker's rail, so any visible line says who wrote
//! it. Settled blocks are cached by (block id, width, revision); only blocks that animate are
//! rendered again each frame, so a long transcript redraws in time proportional to the screen.

use std::collections::HashMap;

use harness::api::runtime_info_t;
use harness::approval::{ApprovalDecision, ApprovalMode, ApprovalSource};
use harness::event::{CancelScope, NoticeLevel, TurnOutcome};
use ratatui::style::{Color, Modifier, Style};

use crate::banner::{LogoRows, LogoWidth, card_input_t, chip, logo_shown, title_card};
use crate::effect::{self, effect_input_t, level_input_t};
use crate::input::Action;
use crate::markdown;
use crate::strip::strip_line;
use crate::text::{
    format_ms, hard_wrap, line_t, sanitize, span_t, spans_width, str_width, truncate, wrap_plain,
    wrap_spans,
};
use crate::theme::theme_t;
use crate::view_model::{
    AttemptStop, BlockKind, ToolStatus, attempt_view_t, block_t, question_view_t, reply_view_t,
    tool_view_t, transcript_t,
};

/// Rows of tool output shown when a block is expanded.
const MaxExpandedRows: usize = 400;
/// Rows of an approval summary shown inline.
const MaxSummaryRows: usize = 16;

pub const RailThick: &str = "▌";
pub const RailThin: &str = "▎";

#[derive(Clone, Debug, PartialEq)]
pub struct row_t {
    pub line: line_t,
    pub rail: Option<(&'static str, Color)>,
    /// Background of the whole row, rail column included.
    pub panel: Option<Color>,
    pub target: Option<Action>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct block_rows_t {
    pub rows: Vec<row_t>,
    /// The label pinned at the top of the viewport while this block's header is scrolled off.
    pub sticky: Option<line_t>,
    /// The block animates and must be rendered again next frame.
    pub live: bool,
}

pub struct render_ctx_t<'a> {
    pub theme: &'a theme_t,
    /// Columns for content, after the margin, the rail and the scrollbar.
    pub width: usize,
    pub clock: u64,
    pub animate: bool,
    pub info: &'a runtime_info_t,
    /// App-clock time the title card's logo reveal started (D56), while it plays.
    pub logo_start: Option<u64>,
    /// Tool blocks start expanded (the `tool_results` setting, D63). A block's own toggle
    /// flips its state from this default.
    pub tools_open: bool,
    /// Blank rows after each block: `BlockGap`, or 0 with the compact density (D63).
    pub gap: usize,
    /// Every accepted link keeps its address after the label (the `link_addresses` setting, D78).
    pub show_addresses: bool,
}

/// Seed of the logo reveal's textures, so every launch draws the same frames.
const LogoSeed: u64 = 0x6E63_6167;

fn header(left: Vec<span_t>, right: Vec<span_t>, width: usize) -> line_t {
    let lw = spans_width(&left);
    let mut line = line_t::new(Vec::new());
    for s in left {
        line.push(s.text, s.style);
    }
    let room = width.saturating_sub(lw + 2);
    if room == 0 || right.is_empty() {
        return line;
    }
    let right_text: String = right.iter().map(|s| s.text.as_str()).collect();
    let rw = str_width(&right_text);
    if rw <= room {
        line.push(" ".repeat(width - lw - rw), Style::new());
        for s in right {
            line.push(s.text, s.style);
        }
    } else {
        let style = right.first().map(|s| s.style).unwrap_or_default();
        let cut = truncate(&right_text, room);
        line.push(" ".repeat(width - lw - str_width(&cut)), Style::new());
        line.push(cut, style);
    }
    line
}

fn rows_from(
    lines: Vec<line_t>,
    rail: Option<(&'static str, Color)>,
    panel: Option<Color>,
    target: Option<Action>,
) -> Vec<row_t> {
    lines
        .into_iter()
        .map(|line| row_t {
            line,
            rail,
            panel,
            target: target.clone(),
        })
        .collect()
}

fn sticky(spans: Vec<span_t>) -> Option<line_t> {
    Some(line_t::new(spans))
}

/// Render one block. Pure apart from reading `transcript.anim` for arrival times.
pub fn render_block(
    block: &block_t,
    transcript: &transcript_t,
    ctx: &render_ctx_t<'_>,
) -> block_rows_t {
    let theme = ctx.theme;
    let w = ctx.width.max(1);
    match &block.kind {
        BlockKind::Banner => {
            let mode = transcript.mode.unwrap_or(ctx.info.approval_mode);
            let card = card_input_t {
                info: ctx.info,
                meta: transcript.meta.as_ref(),
                mode,
            };
            let mut card = title_card(&card, w, theme);
            let mut live = false;
            if let Some(start) = ctx.logo_start
                && ctx.animate
                && logo_shown(w)
            {
                let n = LogoRows.min(card.len());
                let frame =
                    effect::render_reveal(&card[..n], LogoWidth, LogoSeed, start, ctx.clock, theme);
                live = !frame.done;
                card.splice(..n, frame.lines);
            }
            let mut lines = vec![line_t::blank()];
            lines.extend(card);
            block_rows_t {
                rows: rows_from(lines, None, None, None),
                sticky: None,
                live,
            }
        }
        BlockKind::User { text, from_queue } => {
            let rail = Some((RailThick, theme.user));
            let panel = Some(theme.panel);
            let turn = block.turn.unwrap_or(0);
            let mut right = vec![span_t::new(
                format!("turn {turn}"),
                Style::new().fg(theme.fg_faint),
            )];
            if *from_queue {
                right.push(span_t::new(
                    " · sent from queue",
                    Style::new().fg(theme.fg_faint),
                ));
            }
            let mut lines = vec![header(
                vec![span_t::new(
                    "you",
                    Style::new().fg(theme.user).add_modifier(Modifier::BOLD),
                )],
                right,
                w,
            )];
            lines.extend(wrap_plain(
                &sanitize(text, true),
                w,
                Style::new().fg(theme.fg),
            ));
            lines.push(line_t::blank());
            block_rows_t {
                rows: rows_from(lines, rail, panel, Some(Action::SelectBlockId(block.id))),
                sticky: sticky(vec![
                    span_t::new(
                        "you",
                        Style::new().fg(theme.user).add_modifier(Modifier::BOLD),
                    ),
                    span_t::new(format!(" · turn {turn}"), Style::new().fg(theme.fg_dim)),
                ]),
                live: false,
            }
        }
        BlockKind::Reply(reply) => reply_rows(block, reply, transcript, ctx),
        BlockKind::Tool(tool) => tool_rows(block, tool, ctx),
        BlockKind::Question(q) => question_rows(block, q, ctx),
        BlockKind::Cancel {
            scope, levels_done, ..
        } => {
            let text = match scope {
                CancelScope::Call => match levels_done {
                    Some(n) => format!("◌ stopped the call after {n} level pass(es)"),
                    None => "◌ stopped the call".to_string(),
                },
                CancelScope::Tool => "◌ stopped the running tool".to_string(),
                CancelScope::Approval => "◌ dropped the approval request".to_string(),
                CancelScope::Question => "◌ dropped the question".to_string(),
            };
            simple(block, &text, theme.fg_dim, theme.fg_faint, w)
        }
        BlockKind::Error {
            scope,
            name,
            message,
        } => {
            let rail = Some((RailThin, theme.error));
            let scope_name = match scope {
                harness::event::ErrorScope::Backend => "backend",
                harness::event::ErrorScope::Reply => "reply",
                harness::event::ErrorScope::Budget => "budget",
                harness::event::ErrorScope::Store => "store",
                harness::event::ErrorScope::Internal => "internal",
            };
            let mut lines = vec![header(
                vec![span_t::new(
                    format!(
                        "✕ {}",
                        truncate(&sanitize(name, false), w.saturating_sub(4))
                    ),
                    Style::new().fg(theme.error).add_modifier(Modifier::BOLD),
                )],
                vec![span_t::new(
                    format!("{scope_name} error"),
                    Style::new().fg(theme.fg_faint),
                )],
                w,
            )];
            lines.extend(wrap_plain(
                &sanitize(message, true),
                w,
                Style::new().fg(theme.fg_dim),
            ));
            block_rows_t {
                rows: rows_from(lines, rail, None, Some(Action::SelectBlockId(block.id))),
                sticky: sticky(vec![span_t::new("error", Style::new().fg(theme.error))]),
                live: false,
            }
        }
        BlockKind::Notice { level, text } => {
            let (glyph, color) = match level {
                NoticeLevel::Info => ("·", theme.fg_dim),
                NoticeLevel::Warning => ("⚠", theme.approval),
            };
            simple(
                block,
                &format!("{glyph} {}", sanitize(text, true)),
                color,
                color,
                w,
            )
        }
        BlockKind::ModeChange { mode } => {
            let rail = Some((RailThin, theme.fg_faint));
            let mut line = line_t::plain("approval mode is now ", Style::new().fg(theme.fg_dim));
            if *mode == ApprovalMode::Full {
                line.spans.push(chip("full", theme.bg, theme.error));
            } else {
                line.push(
                    mode.name(),
                    Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
                );
            }
            block_rows_t {
                rows: rows_from(
                    vec![line],
                    rail,
                    None,
                    Some(Action::SelectBlockId(block.id)),
                ),
                sticky: None,
                live: false,
            }
        }
        BlockKind::TurnEnd {
            outcome,
            calls,
            passes,
            elapsed_ms,
        } => {
            let (dot, color, word) = match outcome {
                TurnOutcome::Completed => ("●", theme.success, "completed"),
                TurnOutcome::Cancelled => ("◌", theme.approval, "cancelled"),
                TurnOutcome::Failed => ("✕", theme.error, "failed"),
            };
            let turn = block.turn.unwrap_or(0);
            let mut line = line_t::new(vec![
                span_t::new(format!("{dot} "), Style::new().fg(color)),
                span_t::new(
                    format!("end of turn {turn}"),
                    Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
                ),
                span_t::new(format!(" · {word}"), Style::new().fg(color)),
            ]);
            let mut tail = format!(" · {calls} call(s) · {passes} pass(es)");
            if let Some(ms) = elapsed_ms {
                tail.push_str(&format!(" · {}", format_ms(*ms)));
            }
            line.push(
                truncate(&tail, w.saturating_sub(line.width())),
                Style::new().fg(theme.fg_dim),
            );
            line.fill = Some(theme.side);
            block_rows_t {
                rows: vec![row_t {
                    line,
                    rail: None,
                    panel: Some(theme.side),
                    target: Some(Action::SelectBlockId(block.id)),
                }],
                sticky: None,
                live: false,
            }
        }
    }
}

fn simple(block: &block_t, text: &str, fg: Color, rail: Color, w: usize) -> block_rows_t {
    let lines = wrap_plain(text, w, Style::new().fg(fg));
    block_rows_t {
        rows: rows_from(
            lines,
            Some((RailThin, rail)),
            None,
            Some(Action::SelectBlockId(block.id)),
        ),
        sticky: None,
        live: false,
    }
}

fn model_name(transcript: &transcript_t, ctx: &render_ctx_t<'_>) -> String {
    let id = transcript
        .meta
        .as_ref()
        .map(|m| m.model_id.as_str())
        .unwrap_or(ctx.info.identity.id.as_str());
    truncate(&sanitize(id, false), 32)
}

fn simulated(transcript: &transcript_t, ctx: &render_ctx_t<'_>) -> bool {
    ctx.info.identity.simulated() || transcript.meta.as_ref().is_some_and(|m| m.simulated)
}

fn attempt_issue(a: &attempt_view_t) -> String {
    a.end
        .as_ref()
        .and_then(|e| e.issues.first().cloned())
        .unwrap_or_else(|| "the reply did not parse".to_string())
}

fn reply_rows(
    block: &block_t,
    reply: &reply_view_t,
    transcript: &transcript_t,
    ctx: &render_ctx_t<'_>,
) -> block_rows_t {
    let theme = ctx.theme;
    let w = ctx.width.max(1);
    let sim = simulated(transcript, ctx);
    let rail = Some((RailThin, theme.agent));
    let target = Some(if block.expandable() {
        Action::ToggleBlock(block.id)
    } else {
        Action::SelectBlockId(block.id)
    });
    let mut lines: Vec<(line_t, Option<Action>)> = Vec::new();
    let last = reply.attempts.last();
    let running = last.is_some_and(attempt_view_t::running);

    let mut left = vec![span_t::new(
        "ncagent",
        Style::new().fg(theme.agent).add_modifier(Modifier::BOLD),
    )];
    if sim {
        left.push(span_t::new(" · demo model", Style::new().fg(theme.fg_dim)));
    }
    let mut right_text = format!(
        "turn {} · call {}",
        reply.turn,
        reply.call.saturating_add(1)
    );
    if let Some(a) = last {
        if a.attempt > 0 {
            right_text.push_str(&format!(" · attempt {}", a.attempt.saturating_add(1)));
        }
        if running {
            let k = a.level_count.max(1);
            let next = (a.levels.len() as u32).saturating_add(1).min(k);
            right_text.push_str(&format!(" · level {next}/{k}"));
        }
    }
    lines.push((
        header(
            left,
            vec![span_t::new(right_text, Style::new().fg(theme.fg_faint))],
            w,
        ),
        None,
    ));

    // Earlier attempts collapse to one line each.
    let count = reply.attempts.len();
    for a in reply.attempts.iter().take(count.saturating_sub(1)) {
        let text = match (&a.end, &a.stop) {
            (Some(e), _) if !e.valid => format!(
                "✕ attempt {} invalid: {} · resampled",
                a.attempt.saturating_add(1),
                sanitize(&attempt_issue(a), false)
            ),
            (Some(_), _) => format!("· attempt {} replaced", a.attempt.saturating_add(1)),
            (None, _) => format!("◌ attempt {} did not finish", a.attempt.saturating_add(1)),
        };
        for l in wrap_plain(
            &text,
            w,
            Style::new().fg(theme.error).add_modifier(Modifier::DIM),
        ) {
            lines.push((l, None));
        }
        if block.expanded
            && let Some(e) = &a.end
        {
            for row in hard_wrap(&sanitize(&e.text, true), w)
                .into_iter()
                .take(MaxExpandedRows)
            {
                lines.push((line_t::plain(row, Style::new().fg(theme.fg_faint)), None));
            }
        }
    }

    let mut live = false;
    if let Some(a) = last {
        let key = (block.id, a.attempt);
        let anim = transcript.anim.get(&key);
        let levels: Vec<level_input_t> = a
            .levels
            .iter()
            .enumerate()
            .map(|(i, l)| level_input_t {
                level: l.level,
                level_len: l.level_len,
                draft: l.draft.clone(),
                arrived_ms: anim.and_then(|t| t.levels_ms.get(i).copied().flatten()),
            })
            .collect();
        let final_lines: Option<Vec<line_t>> = a.end.as_ref().map(|e| {
            if !e.valid {
                hard_wrap(&sanitize(&e.text, true), w)
                    .into_iter()
                    .take(MaxExpandedRows)
                    .map(|r| line_t::plain(r, Style::new().fg(theme.fg_faint)))
                    .collect()
            } else if e.display.trim().is_empty() {
                Vec::new()
            } else {
                markdown::render_with(&e.display, w, theme, ctx.show_addresses)
            }
        });
        let stopped = a.stop.is_some() && a.end.is_none();
        if !stopped {
            let input = effect_input_t {
                seed: a.seed,
                level_count: a.level_count.max(1),
                levels: &levels,
                started_ms: anim.and_then(|t| t.started_ms),
                end: final_lines
                    .as_deref()
                    .map(|f| (f, anim.and_then(|t| t.ended_ms))),
            };
            let frame = effect::render_effect(&input, ctx.clock, w, theme, ctx.animate);
            live = !frame.done;
            let invalid_done = frame.done && a.end.as_ref().is_some_and(|e| !e.valid);
            if invalid_done {
                let text = format!(
                    "✕ attempt {} invalid: {}",
                    a.attempt.saturating_add(1),
                    sanitize(&attempt_issue(a), false)
                );
                for l in wrap_plain(&text, w, Style::new().fg(theme.error)) {
                    lines.push((l, None));
                }
                if block.expanded {
                    for l in final_lines.iter().flatten() {
                        lines.push((l.clone(), None));
                    }
                } else {
                    lines.push((
                        line_t::plain(
                            "enter or click shows the raw reply",
                            Style::new().fg(theme.fg_faint),
                        ),
                        None,
                    ));
                }
            } else {
                for l in frame.lines {
                    lines.push((l, None));
                }
            }
        }
        if running {
            let strip = strip_line(
                &a.levels,
                a.level_count,
                true,
                ctx.clock,
                ctx.animate,
                w.saturating_sub(2),
                theme,
            );
            lines.push((strip, Some(Action::Cancel)));
            let legend = if sim {
                "pass times: simulator wall clock · esc or click stops after this pass"
            } else {
                "pass times: measured wall clock · esc or click stops after this pass"
            };
            lines.push((
                line_t::plain(truncate(legend, w), Style::new().fg(theme.fg_faint)),
                Some(Action::Cancel),
            ));
        }
        match &a.stop {
            Some(AttemptStop::Cancelled { levels_done }) => {
                let text = match levels_done {
                    Some(n) => format!("◌ stopped after {n} of {} level pass(es)", a.level_count),
                    None => "◌ stopped".to_string(),
                };
                lines.push((line_t::plain(text, Style::new().fg(theme.fg_dim)), None));
            }
            Some(AttemptStop::Failed { name }) => {
                lines.push((
                    line_t::plain(
                        truncate(&format!("✕ call failed · {}", sanitize(name, false)), w),
                        Style::new().fg(theme.error),
                    ),
                    None,
                ));
            }
            Some(AttemptStop::Interrupted) => {
                lines.push((
                    line_t::plain("◌ interrupted", Style::new().fg(theme.fg_dim)),
                    None,
                ));
            }
            None => {}
        }
        if let Some(e) = &a.end
            && !live
        {
            if e.truncated {
                let text = "⚠ the reply used every position without an end marker · continuation of truncated replies (D11) is not implemented";
                for l in wrap_plain(text, w, Style::new().fg(theme.approval)) {
                    lines.push((l, None));
                }
            }
            if e.valid && (!e.calls.is_empty() || e.asks > 0) {
                let mut parts: Vec<String> = e.calls.iter().map(|c| sanitize(c, false)).collect();
                if e.asks > 0 {
                    parts.push("a question".to_string());
                }
                let text = format!("→ asks for {}", parts.join(", "));
                for l in wrap_plain(
                    &text,
                    w,
                    Style::new().fg(theme.tool).add_modifier(Modifier::ITALIC),
                ) {
                    lines.push((l, None));
                }
            }
            let wall = if sim {
                "simulated wall time"
            } else {
                "wall time"
            };
            let meta = format!(
                " {} · {} pass(es) · {} {wall}",
                model_name(transcript, ctx),
                e.passes,
                format_ms(a.elapsed_ms())
            );
            let mut row = line_t::new(vec![span_t::new("▪", Style::new().fg(theme.agent))]);
            row.push(
                truncate(&meta, w.saturating_sub(1)),
                Style::new().fg(theme.fg_faint),
            );
            lines.push((row, None));
        }
    }
    let rows = lines
        .into_iter()
        .map(|(line, t)| row_t {
            line,
            rail,
            panel: None,
            target: t.or_else(|| target.clone()),
        })
        .collect();
    block_rows_t {
        rows,
        sticky: sticky(vec![
            span_t::new(
                "ncagent",
                Style::new().fg(theme.agent).add_modifier(Modifier::BOLD),
            ),
            span_t::new(
                format!(
                    " · turn {} · call {}",
                    reply.turn,
                    reply.call.saturating_add(1)
                ),
                Style::new().fg(theme.fg_dim),
            ),
        ]),
        live,
    }
}

fn args_summary(tool: &tool_view_t) -> String {
    tool.args
        .iter()
        .take(3)
        .map(|a| {
            format!(
                "{}={}",
                sanitize(&a.name, false),
                truncate(&sanitize(&a.value, false), 40)
            )
        })
        .collect::<Vec<_>>()
        .join("  ")
}

fn tool_rows(block: &block_t, tool: &tool_view_t, ctx: &render_ctx_t<'_>) -> block_rows_t {
    let theme = ctx.theme;
    let w = ctx.width.max(1);
    // The block's toggle flips it from the default of the `tool_results` setting.
    let expanded = block.expanded != ctx.tools_open;
    let rail = Some((RailThin, theme.tool));
    let target = Some(Action::ToggleBlock(block.id));
    let spinner = ["◐", "◓", "◑", "◒"];
    let (status_text, status_color) = match tool.status {
        ToolStatus::Pending => ("○ pending".to_string(), theme.fg_faint),
        ToolStatus::AwaitingApproval => ("◇ awaiting approval".to_string(), theme.approval),
        ToolStatus::Running => {
            let frame = if ctx.animate {
                spinner[((ctx.clock / 120) % 4) as usize]
            } else {
                "◐"
            };
            (format!("{frame} running"), theme.tool)
        }
        ToolStatus::Ok => ("● ok".to_string(), theme.success),
        ToolStatus::Failed => ("✕ failed".to_string(), theme.error),
        ToolStatus::Denied => ("⊘ denied".to_string(), theme.error),
        ToolStatus::Cancelled => ("◌ cancelled".to_string(), theme.fg_faint),
    };
    let mut right = vec![span_t::new(status_text, Style::new().fg(status_color))];
    if tool.output.as_ref().is_some_and(|o| o.simulated) {
        right.push(span_t::new(" · simulated", Style::new().fg(theme.fg_faint)));
    }
    let name = truncate(&sanitize(&tool.tool, false), 32);
    let left = vec![
        span_t::new("◈ ", Style::new().fg(theme.tool)),
        span_t::new(
            name.clone(),
            Style::new().fg(theme.tool).add_modifier(Modifier::BOLD),
        ),
        span_t::new(
            format!(" · {}", tool.class.name()),
            Style::new().fg(theme.tool_dim),
        ),
    ];
    let mut lines = vec![header(left, right, w)];
    let args = args_summary(tool);
    if !args.is_empty() {
        lines.push(line_t::plain(
            truncate(&args, w),
            Style::new().fg(theme.fg_dim),
        ));
    }
    if let Some(req) = &tool.approval {
        for l in wrap_spans(
            &[span_t::new(
                format!("asked because {}", sanitize(&req.reason, false)),
                Style::new().fg(theme.approval),
            )],
            w,
            &[span_t::new("◇ ", Style::new().fg(theme.approval))],
            &[span_t::new("  ", Style::new())],
        ) {
            lines.push(l);
        }
        // While pending, the docked approval widget shows the summary; here it shows on expand.
        let show_summary = expanded;
        if show_summary {
            let inner = w.saturating_sub(2).max(1);
            let rows = hard_wrap(&sanitize(&req.summary, true), inner);
            let total = rows.len();
            let limit = if expanded {
                MaxExpandedRows
            } else {
                MaxSummaryRows
            };
            for row in rows.into_iter().take(limit) {
                let pad = inner.saturating_sub(str_width(&row));
                let mut l = line_t::plain(
                    format!(" {row}{} ", " ".repeat(pad)),
                    Style::new().fg(theme.fg).bg(theme.inset),
                );
                l.fill = None;
                lines.push(l);
            }
            if total > limit {
                lines.push(line_t::plain(
                    format!("+{} more line(s)", total - limit),
                    Style::new().fg(theme.fg_faint),
                ));
            }
        }
    }
    if let Some((decision, source)) = tool.decision
        && (tool.approval.is_some() || decision == ApprovalDecision::Deny)
    {
        let (text, color) = match (decision, source) {
            (ApprovalDecision::Deny, ApprovalSource::User) => ("✕ denied by you", theme.error),
            (ApprovalDecision::Deny, _) => ("✕ refused by the approval mode", theme.error),
            (ApprovalDecision::Once, ApprovalSource::User) => {
                ("✓ allowed once by you", theme.success)
            }
            (ApprovalDecision::Session, ApprovalSource::User) => {
                ("✓ allowed for this session by you", theme.success)
            }
            (_, ApprovalSource::Mode) => ("✓ allowed by the approval mode", theme.success),
            (_, ApprovalSource::SessionGrant) => {
                ("✓ allowed by an earlier session grant", theme.success)
            }
        };
        lines.push(line_t::plain(text, Style::new().fg(color)));
    }
    if let Some(out) = &tool.output {
        let clean = sanitize(&out.text, true);
        let inner = w.saturating_sub(2).max(1);
        let all = if clean.is_empty() {
            vec!["(no output)".to_string()]
        } else {
            hard_wrap(&clean, inner)
        };
        let total = all.len();
        let shown = if expanded {
            total.min(MaxExpandedRows)
        } else {
            1
        };
        let fg = if out.ok { theme.fg } else { theme.error };
        for row in all.iter().take(shown) {
            let pad = inner.saturating_sub(str_width(row));
            lines.push(line_t::plain(
                format!(" {row}{} ", " ".repeat(pad)),
                Style::new().fg(fg).bg(theme.inset),
            ));
        }
        let hidden = total.saturating_sub(shown);
        let mut tail = Vec::new();
        if hidden > 0 {
            let verb = if expanded {
                "not shown"
            } else {
                "· enter or click to expand"
            };
            tail.push(format!("+{hidden} line(s) {verb}"));
        } else if expanded && total > 1 {
            tail.push("enter or click to fold".to_string());
        }
        if out.truncated {
            tail.push("cut at the result limit by the runner".to_string());
        }
        if !tail.is_empty() {
            lines.push(line_t::plain(
                truncate(&tail.join(" · "), w),
                Style::new().fg(theme.fg_faint),
            ));
        }
    }
    if let Some((kind, message)) = &tool.error {
        let text = format!("✕ {}: {}", kind.name(), sanitize(message, true));
        lines.extend(wrap_plain(&text, w, Style::new().fg(theme.error)));
    }
    block_rows_t {
        rows: rows_from(lines, rail, None, target),
        sticky: sticky(vec![
            span_t::new(
                "tool",
                Style::new().fg(theme.tool).add_modifier(Modifier::BOLD),
            ),
            span_t::new(format!(" · {name}"), Style::new().fg(theme.fg_dim)),
        ]),
        live: tool.status == ToolStatus::Running && ctx.animate,
    }
}

fn question_rows(block: &block_t, q: &question_view_t, ctx: &render_ctx_t<'_>) -> block_rows_t {
    let theme = ctx.theme;
    let w = ctx.width.max(1);
    let rail = Some((RailThin, theme.question));
    let status = if q.answer.is_some() {
        ("answered", theme.success)
    } else if q.dropped {
        ("dropped", theme.fg_faint)
    } else {
        ("waiting for you", theme.question)
    };
    let mut lines = vec![header(
        vec![span_t::new(
            "? ncagent asks",
            Style::new().fg(theme.question).add_modifier(Modifier::BOLD),
        )],
        vec![span_t::new(status.0, Style::new().fg(status.1))],
        w,
    )];
    lines.extend(wrap_plain(
        &sanitize(&q.question, true),
        w,
        Style::new().fg(theme.fg),
    ));
    let chosen = q.answer.as_ref().and_then(|(_, i)| *i);
    // While pending, the options are buttons in the dock; the block lists them once answered.
    let options: &[String] = if q.pending() { &[] } else { &q.options };
    for (i, option) in options.iter().enumerate() {
        let picked = chosen == Some(i as u32);
        let mark = if picked { "✓" } else { " " };
        let style = if picked {
            Style::new().fg(theme.success).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.fg_dim)
        };
        let first = vec![span_t::new(format!("{mark} {:>2}  ", i + 1), style)];
        let rest = vec![span_t::new("      ", Style::new())];
        lines.extend(wrap_spans(
            &[span_t::new(sanitize(option, false), style)],
            w,
            &first,
            &rest,
        ));
    }
    if let Some((answer, None)) = &q.answer {
        lines.extend(wrap_spans(
            &[span_t::new(
                sanitize(answer, true),
                Style::new().fg(theme.fg),
            )],
            w,
            &[span_t::new("↳ ", Style::new().fg(theme.success))],
            &[span_t::new("  ", Style::new())],
        ));
    }
    if q.pending() {
        lines.push(line_t::plain(
            "answer below: a number, or type your own",
            Style::new().fg(theme.fg_faint),
        ));
    }
    block_rows_t {
        rows: rows_from(lines, rail, None, Some(Action::SelectBlockId(block.id))),
        sticky: sticky(vec![span_t::new(
            "ncagent asks",
            Style::new().fg(theme.question).add_modifier(Modifier::BOLD),
        )]),
        live: false,
    }
}

/// Blank rows after each block.
pub const BlockGap: usize = 1;

struct cached_t {
    width: usize,
    revision: u64,
    animate: bool,
    tools_open: bool,
    show_addresses: bool,
    rows: block_rows_t,
}

/// Rendered rows of a transcript with a per-block cache and row offsets.
#[derive(Default)]
pub struct transcript_view_t {
    cache: HashMap<u64, cached_t>,
    /// Start row of each block, by block index.
    pub starts: Vec<usize>,
    pub total: usize,
    /// Blocks rendered in the last `prepare` (cache misses), for tests.
    pub rendered: usize,
}

impl transcript_view_t {
    pub fn prepare(&mut self, transcript: &transcript_t, ctx: &render_ctx_t<'_>) {
        self.starts.clear();
        self.total = 0;
        self.rendered = 0;
        for block in &transcript.blocks {
            let reuse = matches!(self.cache.get(&block.id), Some(c) if c.width == ctx.width
                && c.revision == block.revision
                && c.animate == ctx.animate
                && c.tools_open == ctx.tools_open
                && c.show_addresses == ctx.show_addresses
                && !c.rows.live
                && !matches!(block.kind, BlockKind::Banner));
            if !reuse {
                let rows = render_block(block, transcript, ctx);
                self.rendered += 1;
                self.cache.insert(
                    block.id,
                    cached_t {
                        width: ctx.width,
                        revision: block.revision,
                        animate: ctx.animate,
                        tools_open: ctx.tools_open,
                        show_addresses: ctx.show_addresses,
                        rows,
                    },
                );
            }
            let height = self
                .cache
                .get(&block.id)
                .map(|c| c.rows.rows.len())
                .unwrap_or(0);
            self.starts.push(self.total);
            self.total = self.total.saturating_add(height).saturating_add(ctx.gap);
        }
    }

    pub fn rows_of(&self, id: u64) -> Option<&block_rows_t> {
        self.cache.get(&id).map(|c| &c.rows)
    }

    /// The block index that owns global row `row`.
    pub fn block_at(&self, row: usize) -> Option<usize> {
        if self.starts.is_empty() {
            return None;
        }
        let i = self.starts.partition_point(|s| *s <= row);
        i.checked_sub(1)
    }

    /// Whether any block animates; the loop draws faster while it does.
    pub fn any_live(&self, transcript: &transcript_t) -> bool {
        transcript
            .blocks
            .iter()
            .rev()
            .take(64)
            .any(|b| self.cache.get(&b.id).is_some_and(|c| c.rows.live))
    }
}
