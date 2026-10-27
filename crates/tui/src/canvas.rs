/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Bounds-checked drawing into a ratatui `Buffer`. Every write goes through `cell_mut`, so a
//! position outside the buffer is skipped rather than a panic, whatever the caller computed.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::text::{clusters, line_t};

/// Paint a rectangle with spaces in `style`.
pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.reset();
                cell.set_symbol(" ");
                cell.set_style(style);
            }
        }
    }
}

/// Set only the background of a rectangle, keeping symbols.
pub fn tint(buf: &mut Buffer, area: Rect, bg: Color) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_bg(bg);
            }
        }
    }
}

/// Write `text` from `x` on row `y`, stopping before column `max_x`. Returns the next column.
/// Text must already be sanitized; control clusters are skipped regardless.
pub fn put_str(buf: &mut Buffer, x: u16, y: u16, max_x: u16, text: &str, style: Style) -> u16 {
    let max_x = max_x.min(buf.area.right());
    let mut x = x;
    for c in clusters(text) {
        if c.width == 0 {
            continue;
        }
        let w = c.width as u16;
        if x.saturating_add(w) > max_x {
            break;
        }
        let symbol = text.get(c.start..c.end).unwrap_or(" ");
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(symbol);
            cell.set_style(style);
        }
        for k in 1..w {
            if let Some(cell) = buf.cell_mut((x.saturating_add(k), y)) {
                cell.reset();
                cell.set_style(style);
            }
        }
        x = x.saturating_add(w);
    }
    x
}

/// Write a styled line. `base` is patched under each span's own style. A line `fill` paints
/// from `x` to `max_x` first.
pub fn put_line(buf: &mut Buffer, x: u16, y: u16, max_x: u16, line: &line_t, base: Style) -> u16 {
    if let Some(fill_bg) = line.fill {
        let width = max_x.saturating_sub(x);
        fill(buf, Rect::new(x, y, width, 1), base.bg(fill_bg));
    }
    let mut cx = x;
    for span in &line.spans {
        let mut style = base.patch(span.style);
        if span.style.bg.is_none()
            && let Some(fill_bg) = line.fill
        {
            style = style.bg(fill_bg);
        }
        cx = put_str(buf, cx, y, max_x, &span.text, style);
        if cx >= max_x {
            break;
        }
    }
    cx
}

/// Right-align `text` so it ends at `right` (exclusive). Returns the start column.
pub fn put_str_right(
    buf: &mut Buffer,
    right: u16,
    y: u16,
    min_x: u16,
    text: &str,
    style: Style,
) -> u16 {
    let w = crate::text::str_width(text) as u16;
    let start = right.saturating_sub(w).max(min_x);
    put_str(buf, start, y, right, text, style);
    start
}

/// Rows of a buffer as plain strings, for tests and diagnostics.
pub fn buffer_lines(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    let mut out = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut row = String::new();
        let mut skip = 0usize;
        for x in area.left()..area.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            if let Some(cell) = buf.cell((x, y)) {
                let symbol = cell.symbol();
                row.push_str(symbol);
                skip = crate::text::str_width(symbol).saturating_sub(1);
            }
        }
        out.push(row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_are_clipped_not_panicking() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 2));
        put_str(&mut buf, 3, 0, 100, "hello", Style::new());
        put_str(&mut buf, 0, 9, 5, "far", Style::new());
        put_str(
            &mut buf,
            u16::MAX - 1,
            u16::MAX,
            u16::MAX,
            "edge",
            Style::new(),
        );
        fill(&mut buf, Rect::new(4, 1, 100, 100), Style::new());
        assert_eq!(buffer_lines(&buf), vec!["   he", "     "]);
    }

    #[test]
    fn wide_glyph_does_not_split() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
        let next = put_str(&mut buf, 0, 0, 3, "日本", Style::new());
        assert_eq!(next, 2);
        assert_eq!(buffer_lines(&buf), vec!["日 "]);
    }

    #[test]
    fn right_aligned() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 1));
        put_str_right(&mut buf, 6, 0, 0, "ab", Style::new());
        assert_eq!(buffer_lines(&buf), vec!["    ab"]);
    }
}
