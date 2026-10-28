/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The level strip: one cell per level pass, filled as passes finish, each labelled with the
//! pass time the backend measured. The running pass pulses. Tone cells, no frames.

use ratatui::style::{Modifier, Style};

use crate::text::{fit, line_t, span_t};
use crate::theme::{mix, theme_t};
use crate::view_model::level_view_t;

/// Columns of one cell for `level_count` cells in `width` columns (with a one-column gap).
pub fn cell_width(level_count: u32, width: usize) -> usize {
    let k = (level_count.max(1) as usize).min(64);
    let per = width / k;
    per.saturating_sub(1).clamp(1, 8)
}

/// The strip as a line of `width` columns. `running` pulses the next cell.
pub fn strip_line(
    levels: &[level_view_t],
    level_count: u32,
    running: bool,
    clock: u64,
    animate: bool,
    width: usize,
    theme: &theme_t,
) -> line_t {
    let k = level_count.max(levels.len() as u32).max(1);
    let shown = (k as usize).min(64);
    let cw = cell_width(k, width);
    let mut line = line_t::blank();
    for i in 0..shown {
        if i > 0 {
            line.push(" ", Style::new());
        }
        let done = levels.get(i);
        let (label, style) = match done {
            Some(level) => {
                let ms = level.elapsed_ms.min(99_999);
                let text = if cw >= 3 {
                    ms.to_string()
                } else {
                    "▪".to_string()
                };
                let shade = (i as f32 + 1.0) / shown as f32;
                let bg = mix(theme.ice_deep, theme.agent, 0.15 + 0.35 * shade);
                (
                    text,
                    Style::new()
                        .fg(theme.ice)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                )
            }
            None if running && i == levels.len() => {
                let t = if animate {
                    ((clock as f32 / 260.0).sin() * 0.5 + 0.5) * 0.8
                } else {
                    0.4
                };
                let bg = mix(theme.inset, theme.ice_deep, t);
                ("…".to_string(), Style::new().fg(theme.ice).bg(bg))
            }
            None => (
                "".to_string(),
                Style::new().fg(theme.fg_faint).bg(theme.inset),
            ),
        };
        let text = if label.chars().count() >= cw {
            fit(&label, cw)
        } else {
            let pad = cw - label.chars().count();
            format!(
                "{}{}{}",
                " ".repeat(pad / 2),
                label,
                " ".repeat(pad - pad / 2)
            )
        };
        line.spans.push(span_t::new(text, style));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lv(level: u32, ms: u64) -> level_view_t {
        level_view_t {
            level,
            level_count: 5,
            level_len: 8,
            elapsed_ms: ms,
            draft: None,
        }
    }

    #[test]
    fn cells_show_measured_times() {
        let t = theme_t::dark();
        let line = strip_line(&[lv(0, 412), lv(1, 380)], 5, true, 0, false, 40, &t);
        let text = line.text();
        assert!(text.contains("412"));
        assert!(text.contains("380"));
        assert!(text.contains('…'));
        assert!(line.width() <= 40);
    }

    #[test]
    fn narrow_and_odd_counts() {
        let t = theme_t::dark();
        for width in [0usize, 1, 5, 12, 200] {
            for k in [0u32, 1, 4, 5, 300] {
                let line = strip_line(&[lv(0, 99_999_999)], k, true, 5, true, width, &t);
                assert!(!line.spans.is_empty());
            }
        }
        assert_eq!(cell_width(5, 40), 7);
        assert_eq!(cell_width(4, 12), 2);
    }
}
