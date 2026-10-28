/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Links as terminal hyperlinks (OSC 8, PROJECT.md D77), written as an overlay after each frame.
//!
//! A ratatui cell cannot carry OSC 8 safely, so the frame is drawn as usual and the visible
//! links are listed with their rectangles. After the frame is flushed, each link is written
//! again over itself: a cursor move, the OSC 8 open with its address, the same cells with the
//! same styles, the OSC 8 close and a style reset. The terminal then knows those cells link to
//! the address and opens it on its own modifier+click.
//!
//! ratatui writes only the cells that changed since the last frame. A link written in the last
//! frame whose cells, place and address are the same is therefore still a hyperlink and is not
//! written again; a link whose cells changed (scrolled, restyled, uncovered) is, so no cell
//! keeps a stale hyperlink. A link under a toast, a menu, a dialog or any other overlay is
//! skipped until it shows again. A resize or a full redraw writes every link again.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::links::safe_url;
use crate::text::{line_t, str_width};

/// One run of linked text in a block's rows: a row, its start column and width, the address,
/// and which link of the block it is (a link wrapped over two rows is one link).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct link_run_t {
    pub row: usize,
    pub col: u16,
    pub width: u16,
    pub url: String,
    pub index: usize,
}

/// The linked runs of `rows`, in order.
pub fn link_runs<'a>(rows: impl Iterator<Item = &'a line_t>) -> Vec<link_run_t> {
    let mut out: Vec<link_run_t> = Vec::new();
    let mut count = 0usize;
    // The address of the run that ended the previous text, if it ran to the end of its row.
    let mut open: Option<String> = None;
    for (row, line) in rows.enumerate() {
        let mut col = 0u16;
        let mut last_link: Option<String> = None;
        let mut first = true;
        for span in &line.spans {
            let w = str_width(&span.text) as u16;
            match &span.link {
                Some(url) => {
                    let continues = (first && open.as_deref() == Some(url.as_str()))
                        || last_link.as_deref() == Some(url.as_str());
                    if !continues {
                        count += 1;
                    }
                    match out.last_mut() {
                        Some(prev)
                            if prev.row == row
                                && prev.url == *url
                                && prev.col + prev.width == col =>
                        {
                            prev.width += w;
                        }
                        _ => out.push(link_run_t {
                            row,
                            col,
                            width: w,
                            url: url.clone(),
                            index: count.saturating_sub(1),
                        }),
                    }
                    last_link = Some(url.clone());
                }
                None => {
                    if !span.text.trim().is_empty() || !first {
                        last_link = None;
                    }
                }
            }
            if !span.text.is_empty() {
                first = false;
            }
            col = col.saturating_add(w);
        }
        open = last_link;
    }
    out
}

/// A link drawn in this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct link_rect_t {
    pub rect: Rect,
    pub url: String,
    /// The transcript block and its link index, for the keyboard.
    pub block: u64,
    pub index: usize,
}

/// One link as it was written: its place, address and cells.
#[derive(Clone, Debug, PartialEq, Eq)]
struct written_t {
    rect: Rect,
    url: String,
    cells: Vec<(String, Style)>,
}

/// What the overlay wrote in the last frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct link_overlay_t {
    area: Rect,
    written: Vec<written_t>,
}

impl link_overlay_t {
    /// Forget what was written: after a resize or a full redraw every link is written again.
    pub fn reset(&mut self) {
        self.written.clear();
    }

    /// The bytes that turn this frame's visible links into hyperlinks. `covers` are the
    /// overlays drawn over the transcript. Empty when `enabled` is off.
    pub fn emit(
        &mut self,
        buf: &Buffer,
        links: &[link_rect_t],
        covers: &[Rect],
        enabled: bool,
    ) -> Vec<u8> {
        if !enabled {
            self.written.clear();
            return Vec::new();
        }
        if buf.area != self.area {
            self.area = buf.area;
            self.written.clear();
        }
        let mut out = Vec::new();
        let mut now: Vec<written_t> = Vec::new();
        for link in links {
            let rect = link.rect.intersection(buf.area);
            if rect.width == 0 || rect.height == 0 || now.iter().any(|w| w.rect == rect) {
                continue;
            }
            if covers.iter().any(|c| c.intersects(rect)) {
                continue;
            }
            // The address was checked when the Markdown was read; check it again, since it
            // goes into an escape sequence.
            let Some(url) = safe_url(&link.url) else {
                continue;
            };
            let cells = read_cells(buf, rect);
            let item = written_t { rect, url, cells };
            if !self.written.contains(&item) {
                write_link(&mut out, &item);
            }
            now.push(item);
        }
        self.written = now;
        out
    }
}

fn read_cells(buf: &Buffer, rect: Rect) -> Vec<(String, Style)> {
    let mut cells = Vec::with_capacity(usize::from(rect.width));
    let mut x = rect.x;
    while x < rect.right() {
        let Some(cell) = buf.cell((x, rect.y)) else {
            break;
        };
        let symbol = cell.symbol().to_string();
        let w = str_width(&symbol).max(1) as u16;
        cells.push((symbol, cell.style()));
        x = x.saturating_add(w);
    }
    cells
}

/// The OSC 8 opening sequence for `url`.
pub fn osc8_open(url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\")
}

/// The OSC 8 closing sequence.
pub const Osc8Close: &str = "\x1b]8;;\x1b\\";

fn write_link(out: &mut Vec<u8>, item: &written_t) {
    let mut s = format!("\x1b[{};{}H", item.rect.y + 1, item.rect.x + 1);
    s.push_str(&osc8_open(&item.url));
    let mut last: Option<Style> = None;
    for (symbol, style) in &item.cells {
        if last != Some(*style) {
            s.push_str(&sgr(*style));
            last = Some(*style);
        }
        // Cells hold sanitized text only; a control character is never written.
        if symbol.chars().any(char::is_control) {
            s.push(' ');
        } else {
            s.push_str(symbol);
        }
    }
    s.push_str(Osc8Close);
    s.push_str("\x1b[0m");
    out.extend_from_slice(s.as_bytes());
}

/// The SGR sequence that sets exactly `style` from a reset.
pub fn sgr(style: Style) -> String {
    let mut parts: Vec<String> = vec!["0".to_string()];
    let mods = style.add_modifier;
    for (m, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::SLOW_BLINK, "5"),
        (Modifier::RAPID_BLINK, "6"),
        (Modifier::REVERSED, "7"),
        (Modifier::HIDDEN, "8"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if mods.contains(m) {
            parts.push(code.to_string());
        }
    }
    if let Some(fg) = style.fg.and_then(|c| color_code(c, 38)) {
        parts.push(fg);
    }
    if let Some(bg) = style.bg.and_then(|c| color_code(c, 48)) {
        parts.push(bg);
    }
    format!("\x1b[{}m", parts.join(";"))
}

fn color_code(color: Color, base: u8) -> Option<String> {
    let indexed = |i: u8| Some(format!("{base};5;{i}"));
    match color {
        Color::Reset => None,
        Color::Rgb(r, g, b) => Some(format!("{base};2;{r};{g};{b}")),
        Color::Indexed(i) => indexed(i),
        Color::Black => indexed(0),
        Color::Red => indexed(1),
        Color::Green => indexed(2),
        Color::Yellow => indexed(3),
        Color::Blue => indexed(4),
        Color::Magenta => indexed(5),
        Color::Cyan => indexed(6),
        Color::Gray => indexed(7),
        Color::DarkGray => indexed(8),
        Color::LightRed => indexed(9),
        Color::LightGreen => indexed(10),
        Color::LightYellow => indexed(11),
        Color::LightBlue => indexed(12),
        Color::LightMagenta => indexed(13),
        Color::LightCyan => indexed(14),
        Color::White => indexed(15),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::span_t;

    fn linked(text: &str, url: &str) -> span_t {
        let mut s = span_t::new(text, Style::new());
        s.link = Some(url.to_string());
        s
    }

    #[test]
    fn runs_follow_a_link_over_a_wrap_and_count_links() {
        let a = "https://a.example";
        let b = "https://b.example";
        let rows = [
            line_t::new(vec![span_t::new("see ", Style::new()), linked("the ", a)]),
            line_t::new(vec![
                linked("paper", a),
                span_t::new(" and ", Style::new()),
                linked("more", b),
            ]),
            line_t::new(vec![linked("again", a)]),
        ];
        let runs = link_runs(rows.iter());
        let short: Vec<(usize, u16, u16, usize)> = runs
            .iter()
            .map(|r| (r.row, r.col, r.width, r.index))
            .collect();
        assert_eq!(
            short,
            vec![(0, 4, 4, 0), (1, 0, 5, 0), (1, 10, 4, 1), (2, 0, 5, 2)]
        );
    }

    fn buf_with(text: &str, style: Style) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 3));
        crate::canvas::put_str(&mut buf, 2, 1, 20, text, style);
        buf
    }

    fn link(x: u16, w: u16, url: &str) -> link_rect_t {
        link_rect_t {
            rect: Rect::new(x, 1, w, 1),
            url: url.to_string(),
            block: 0,
            index: 0,
        }
    }

    #[test]
    fn one_and_two_links_emit_exact_bytes_once() {
        let style = Style::new()
            .fg(Color::Rgb(1, 2, 3))
            .add_modifier(Modifier::UNDERLINED);
        let buf = buf_with("ab cd", style);
        let mut o = link_overlay_t::default();
        let bytes = o.emit(&buf, &[link(2, 2, "https://a.example")], &[], true);
        let want =
            "\x1b[2;3H\x1b]8;;https://a.example\x1b\\\x1b[0;4;38;2;1;2;3mab\x1b]8;;\x1b\\\x1b[0m";
        assert_eq!(String::from_utf8_lossy(&bytes), want);
        // The same frame again: nothing to write.
        assert!(
            o.emit(&buf, &[link(2, 2, "https://a.example")], &[], true)
                .is_empty()
        );
        // Two links: both, in order.
        let mut o = link_overlay_t::default();
        let bytes = o.emit(
            &buf,
            &[
                link(2, 2, "https://a.example"),
                link(5, 2, "https://b.example/x?y=1"),
            ],
            &[],
            true,
        );
        let text = String::from_utf8_lossy(&bytes).to_string();
        assert_eq!(text.matches("\x1b]8;;https://").count(), 2);
        assert!(text.contains("\x1b[2;6H\x1b]8;;https://b.example/x?y=1\x1b\\"));
        assert_eq!(text.matches(Osc8Close).count(), 2);
    }

    #[test]
    fn hostile_covered_off_and_changed_links() {
        let buf = buf_with("ab", Style::new());
        let mut o = link_overlay_t::default();
        for bad in [
            "https://e.example/\u{1b}]8;;x",
            "https://e.example/\u{7}",
            "javascript:alert(1)",
            "https://e.example/a b",
        ] {
            assert!(
                o.emit(&buf, &[link(2, 2, bad)], &[], true).is_empty(),
                "{bad:?}"
            );
        }
        let good = [link(2, 2, "https://a.example")];
        assert!(
            o.emit(&buf, &good, &[Rect::new(3, 0, 5, 3)], true)
                .is_empty(),
            "covered"
        );
        assert!(o.emit(&buf, &good, &[], false).is_empty(), "off");
        assert!(
            !o.emit(&buf, &good, &[], true).is_empty(),
            "uncovered: written"
        );
        let changed = buf_with("ab", Style::new().fg(Color::Red));
        assert!(
            !o.emit(&changed, &good, &[], true).is_empty(),
            "restyled: written again"
        );
        o.reset();
        assert!(
            !o.emit(&changed, &good, &[], true).is_empty(),
            "after a reset"
        );
    }
}
