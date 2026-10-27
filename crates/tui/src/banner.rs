/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The title card at the top of the transcript: the "ncagent" logo and what the run is.
//!
//! The logo is drawn from hand-built pixel rows, two pixels per terminal row through the half
//! blocks `▀ ▄ █`, so vertical stems are two columns wide and horizontal bars one row tall. That
//! keeps stroke weight even on a typical 1:2 cell. Each row is shaded from white to ice.

use harness::api::runtime_info_t;
use harness::approval::ApprovalMode;
use ratatui::style::{Color, Modifier, Style};

use crate::text::{line_t, sanitize, span_t, str_width, truncate, wrap_spans};
use crate::theme::{mix, theme_t};
use crate::view_model::session_meta_t;

/// Pixel rows of each letter: 6 wide, 12 tall (ascender, x-height of 8, descender).
const LetterN: [&str; 12] = [
    "......", "......", "#####.", "######", "##..##", "##..##", "##..##", "##..##", "##..##",
    "##..##", "......", "......",
];
const LetterC: [&str; 12] = [
    "......", "......", ".#####", "######", "##....", "##....", "##....", "##....", "######",
    ".#####", "......", "......",
];
const LetterA: [&str; 12] = [
    "......", "......", "#####.", "######", "....##", ".#####", "######", "##..##", "######",
    ".#####", "......", "......",
];
const LetterG: [&str; 12] = [
    "......", "......", ".#####", "######", "##..##", "##..##", "######", ".#####", "....##",
    "....##", "######", "#####.",
];
const LetterE: [&str; 12] = [
    "......", "......", ".####.", "######", "##..##", "######", "######", "##....", "######",
    ".#####", "......", "......",
];
const LetterT: [&str; 12] = [
    ".##...", ".##...", "######", "######", ".##...", ".##...", ".##...", ".##...", ".##...",
    "..####", "......", "......",
];

const Word: [&[&str; 12]; 7] = [
    &LetterN, &LetterC, &LetterA, &LetterG, &LetterE, &LetterN, &LetterT,
];

/// Width of the logo in columns.
pub const LogoWidth: usize = 7 * 6 + 6;
pub const LogoRows: usize = 6;

fn pixel(rows: &[&str; 12], y: usize, x: usize) -> bool {
    rows.get(y)
        .and_then(|r| r.as_bytes().get(x))
        .is_some_and(|b| *b == b'#')
}

/// The logo as plain text rows.
pub fn logo_rows() -> Vec<String> {
    let mut out = Vec::with_capacity(LogoRows);
    for row in 0..LogoRows {
        let mut line = String::new();
        for (i, letter) in Word.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            for x in 0..6 {
                let top = pixel(letter, row * 2, x);
                let bottom = pixel(letter, row * 2 + 1, x);
                line.push(match (top, bottom) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                });
            }
        }
        out.push(line.trim_end().to_string());
    }
    out
}

/// A small filled label such as ` DEMO `.
pub fn chip(text: &str, fg: Color, bg: Color) -> span_t {
    span_t::new(
        format!(" {text} "),
        Style::new().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

/// What the card shows, gathered by the caller.
pub struct card_input_t<'a> {
    pub info: &'a runtime_info_t,
    pub meta: Option<&'a session_meta_t>,
    pub mode: ApprovalMode,
}

fn kv(label: &str, value: String, theme: &theme_t) -> Vec<span_t> {
    vec![
        span_t::new(format!("{label:<9}"), Style::new().fg(theme.fg_faint)),
        span_t::new(value, Style::new().fg(theme.fg)),
    ]
}

/// Keep the end of a path, which is the part that tells projects apart.
pub fn short_path(path: &str, max: usize) -> String {
    let clean = sanitize(path, false);
    let w = str_width(&clean);
    if w <= max {
        return clean;
    }
    let chars: Vec<char> = clean.chars().collect();
    let mut out: Vec<char> = Vec::new();
    let mut used = 1usize;
    for c in chars.iter().rev() {
        let cw = str_width(&c.to_string());
        if used + cw > max {
            break;
        }
        out.push(*c);
        used += cw;
    }
    out.reverse();
    format!("…{}", out.into_iter().collect::<String>())
}

fn info_lines(input: &card_input_t<'_>, width: usize, theme: &theme_t) -> Vec<line_t> {
    let info = input.info;
    let caps = &info.capabilities;
    let simulated = info.identity.simulated();
    let value_w = width.saturating_sub(9).max(4);
    let mut out = Vec::new();
    let mut title = line_t::new(vec![
        span_t::new(
            "ncagent",
            Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        span_t::new(
            format!(" {}", truncate(&sanitize(&info.version, false), 16)),
            Style::new().fg(theme.fg_dim),
        ),
    ]);
    if simulated {
        title.spans.push(span_t::new("  ", Style::new()));
        title.spans.push(chip("DEMO", theme.bg, theme.approval));
    }
    out.push(title);
    out.push(line_t::new(kv(
        "model",
        truncate(&sanitize(&info.identity.id, false), value_w),
        theme,
    )));
    out.push(line_t::new(kv(
        "shape",
        truncate(
            &format!(
                "P {} · L {} · K {}",
                caps.prefix_window, caps.reply_len, caps.level_count
            ),
            value_w,
        ),
        theme,
    )));
    out.push(line_t::new(kv(
        "cwd",
        short_path(&info.cwd, value_w),
        theme,
    )));
    let session = input
        .meta
        .map(|m| truncate(&sanitize(&m.session_id, false), 12))
        .unwrap_or_else(|| "starting".to_string());
    let mut mode_line = line_t::new(kv("approval", String::new(), theme));
    mode_line.spans.pop();
    let mode_style = if input.mode == ApprovalMode::Full {
        Style::new()
            .fg(theme.bg)
            .bg(theme.error)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.fg)
    };
    mode_line.push(input.mode.name(), mode_style);
    mode_line.push(
        truncate(&format!(" · session {session}"), value_w.saturating_sub(5)),
        Style::new().fg(theme.fg_dim),
    );
    out.push(mode_line);
    let contract = input
        .meta
        .map(|m| format!(" · contract v{}", m.contract_version))
        .unwrap_or_default();
    let backend = truncate(&format!("{}{contract}", info.identity.kind.name()), value_w);
    out.push(line_t::new(kv("backend", backend, theme)));
    out
}

/// The title card as lines, `width` columns wide.
pub fn title_card(input: &card_input_t<'_>, width: usize, theme: &theme_t) -> Vec<line_t> {
    let width = width.max(1);
    let info = input.info;
    let mut out = Vec::new();
    let side_by_side = width >= LogoWidth + 4 + 34;
    if width >= LogoWidth + 2 {
        let logo = logo_rows();
        let details = if side_by_side {
            info_lines(input, width - LogoWidth - 4, theme)
        } else {
            Vec::new()
        };
        for (row, text) in logo.iter().enumerate() {
            let t = row as f32 / (LogoRows - 1) as f32;
            let color = mix(Color::Rgb(255, 255, 255), Color::Rgb(116, 176, 238), t);
            let mut line = line_t::plain(
                format!("{text:<width$}", width = LogoWidth),
                Style::new().fg(color),
            );
            if let Some(d) = details.get(row) {
                line.push("    ", Style::new());
                for s in &d.spans {
                    line.push(s.text.clone(), s.style);
                }
            }
            out.push(line);
        }
        if !side_by_side {
            out.push(line_t::blank());
            out.extend(info_lines(input, width, theme));
        }
    } else {
        // Narrow: a one-line wordmark, then the details.
        let mut mark = line_t::new(vec![
            span_t::new("▐", Style::new().fg(theme.ice)),
            span_t::new(
                "ncagent",
                Style::new().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ]);
        if info.identity.simulated() {
            mark.push("  ", Style::new());
            let c = chip("DEMO", theme.bg, theme.approval);
            mark.spans.push(c);
        }
        out.push(mark);
        out.push(line_t::blank());
        let mut details = info_lines(input, width, theme);
        if !details.is_empty() {
            details.remove(0);
        }
        out.extend(details);
    }
    out.push(line_t::blank());
    if let Some(notice) = &info.notice {
        let first = vec![
            chip("DEMO", theme.bg, theme.approval),
            span_t::new(" ", Style::new()),
        ];
        let rest = vec![span_t::new("       ", Style::new())];
        let text = sanitize(notice, false);
        out.extend(wrap_spans(
            &[span_t::new(text, Style::new().fg(theme.approval))],
            width,
            if info.identity.simulated() {
                &first
            } else {
                &[]
            },
            if info.identity.simulated() {
                &rest
            } else {
                &[]
            },
        ));
    }
    if let Some(label) = input.meta.and_then(|m| m.label.as_ref())
        && info.notice.as_deref() != Some(label.as_str())
    {
        out.extend(wrap_spans(
            &[span_t::new(
                sanitize(label, false),
                Style::new().fg(theme.fg_dim),
            )],
            width,
            &[],
            &[],
        ));
    }
    if let Some(note) = &info.state_note {
        out.extend(wrap_spans(
            &[
                span_t::new("state  ", Style::new().fg(theme.fg_faint)),
                span_t::new(sanitize(note, false), Style::new().fg(theme.fg_dim)),
            ],
            width,
            &[],
            &[],
        ));
    }
    if !info.hints.is_empty() {
        out.push(line_t::blank());
        out.push(line_t::plain(
            "try typing",
            Style::new()
                .fg(theme.fg_faint)
                .add_modifier(Modifier::ITALIC),
        ));
        let trigger_w = info
            .hints
            .iter()
            .map(|h| str_width(&sanitize(&h.trigger, false)))
            .max()
            .unwrap_or(0)
            .min(14);
        for hint in info.hints.iter().take(12) {
            let trigger = truncate(&sanitize(&hint.trigger, false), trigger_w);
            let pad = trigger_w.saturating_sub(str_width(&trigger));
            let first = vec![
                span_t::new("  ", Style::new()),
                span_t::new(
                    trigger,
                    Style::new().fg(theme.ice).add_modifier(Modifier::BOLD),
                ),
                span_t::new(" ".repeat(pad + 2), Style::new()),
            ];
            let rest = vec![span_t::new(" ".repeat(trigger_w + 4), Style::new())];
            out.extend(wrap_spans(
                &[span_t::new(
                    sanitize(&hint.description, false),
                    Style::new().fg(theme.fg_dim),
                )],
                width,
                &first,
                &rest,
            ));
        }
    }
    out.push(line_t::blank());
    let key = Style::new().fg(theme.fg).add_modifier(Modifier::BOLD);
    let label = Style::new().fg(theme.fg_faint);
    let tips = [
        ("/help", "commands"),
        ("?", "keys"),
        ("tab", "focus"),
        ("ctrl+b", "sidebar"),
        ("esc", "stop"),
    ];
    let mut spans = Vec::new();
    for (i, (k, l)) in tips.iter().enumerate() {
        if i > 0 {
            spans.push(span_t::new("   ", label));
        }
        spans.push(span_t::new(*k, key));
        spans.push(span_t::new(format!(" {l}"), label));
    }
    out.extend(wrap_spans(&spans, width, &[], &[]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_has_six_rows_and_fixed_width() {
        let rows = logo_rows();
        assert_eq!(rows.len(), LogoRows);
        for r in &rows {
            assert!(str_width(r) <= LogoWidth);
        }
        assert!(rows.iter().any(|r| str_width(r) == LogoWidth));
        // Strokes are drawn with blocks only.
        for r in &rows {
            assert!(r.chars().all(|c| matches!(c, ' ' | '█' | '▀' | '▄')));
        }
    }

    #[test]
    fn logo_snapshot() {
        let rows = logo_rows();
        assert_eq!(rows[1], "█████▄ ▄█████ █████▄ ▄█████ ▄████▄ █████▄ ██████");
        assert_eq!(rows[2], "██  ██ ██      ▄▄▄██ ██  ██ ██▄▄██ ██  ██  ██");
    }

    #[test]
    fn short_path_keeps_the_tail() {
        assert_eq!(short_path("/home/dev/ncagent", 40), "/home/dev/ncagent");
        assert_eq!(short_path("/home/dev/ncagent", 8), "…ncagent");
    }
}
