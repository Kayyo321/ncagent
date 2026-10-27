/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Styled lines, display width and word wrapping.
//!
//! Every string that reaches the screen passes through [`sanitize`] first, so model output and
//! tool results can never write a control character or an escape sequence to the terminal.
//! Widths come from `unicode-width`. A cluster is a base character plus the zero-width
//! characters after it, with zero-width-joiner sequences kept together, which matches what a
//! terminal draws closely enough for layout and never splits a combining mark from its base.

use ratatui::style::{Color, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A run of text in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct span_t {
    pub text: String,
    pub style: Style,
}

impl span_t {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        span_t {
            text: text.into(),
            style,
        }
    }
}

/// One display line. `fill` paints the whole row behind the spans (code blocks, tone strips).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct line_t {
    pub spans: Vec<span_t>,
    pub fill: Option<Color>,
}

impl line_t {
    pub fn new(spans: Vec<span_t>) -> Self {
        line_t { spans, fill: None }
    }

    pub fn plain(text: impl Into<String>, style: Style) -> Self {
        line_t::new(vec![span_t::new(text, style)])
    }

    pub fn blank() -> Self {
        line_t::default()
    }

    pub fn with_fill(mut self, fill: Color) -> Self {
        self.fill = Some(fill);
        self
    }

    pub fn width(&self) -> usize {
        self.spans.iter().map(|s| str_width(&s.text)).sum()
    }

    /// The text of the line, without styles.
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn push(&mut self, text: impl Into<String>, style: Style) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut()
            && last.style == style
        {
            last.text.push_str(&text);
            return;
        }
        self.spans.push(span_t::new(text, style));
    }
}

const TabWidth: usize = 4;

/// Remove control characters and terminal escape sequences. Tabs become spaces. Newlines stay
/// unless `keep_newlines` is false, in which case they become spaces.
pub fn sanitize(input: &str, keep_newlines: bool) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push(if keep_newlines { '\n' } else { ' ' }),
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push(if keep_newlines { '\n' } else { ' ' });
                }
            }
            '\t' => out.extend(std::iter::repeat_n(' ', TabWidth)),
            '\u{1b}' => skip_escape(&mut chars),
            c if c.is_control() => {}
            // Bidirectional overrides can reorder what a reader sees; drop them.
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            c => out.push(c),
        }
    }
    out
}

fn skip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            for c in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
        Some(']') | Some('P') | Some('_') | Some('^') => {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '\u{7}' {
                    break;
                }
                if c == '\u{1b}' {
                    if chars.peek() == Some(&'\\') {
                        chars.next();
                    }
                    break;
                }
            }
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

/// A display cluster: byte range and width (0, 1 or 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct cluster_t {
    pub start: usize,
    pub end: usize,
    pub width: usize,
}

/// Split a string into display clusters. Control characters get width 0.
pub fn clusters(s: &str) -> Vec<cluster_t> {
    let mut out: Vec<cluster_t> = Vec::new();
    let mut prev_zwj = false;
    let mut prev_control = true;
    for (i, c) in s.char_indices() {
        let control = c.is_control();
        let w = if control { 0 } else { c.width().unwrap_or(0) };
        let joins = !control && !prev_control && (w == 0 || prev_zwj);
        prev_zwj = c == '\u{200d}';
        prev_control = control;
        match out.last_mut() {
            Some(last) if joins => {
                last.end = i + c.len_utf8();
                let text = s.get(last.start..last.end).unwrap_or("");
                last.width = text.width().min(2);
            }
            _ => out.push(cluster_t {
                start: i,
                end: i + c.len_utf8(),
                width: w.min(2),
            }),
        }
    }
    out
}

pub fn str_width(s: &str) -> usize {
    if s.is_ascii() {
        return s.bytes().filter(|b| !b.is_ascii_control()).count();
    }
    clusters(s).iter().map(|c| c.width).sum()
}

/// Cut a string to `max` columns, ending with `…` when cut.
pub fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for c in clusters(s) {
        if used + c.width > max.saturating_sub(1) {
            break;
        }
        out.push_str(s.get(c.start..c.end).unwrap_or(""));
        used += c.width;
    }
    out.push('…');
    out
}

/// Pad or cut to exactly `width` columns.
pub fn fit(s: &str, width: usize) -> String {
    let mut out = truncate(s, width);
    let w = str_width(&out);
    out.extend(std::iter::repeat_n(' ', width.saturating_sub(w)));
    out
}

pub fn spans_width(spans: &[span_t]) -> usize {
    spans.iter().map(|s| str_width(&s.text)).sum()
}

struct wrap_state_t<'a> {
    lines: Vec<line_t>,
    cur: line_t,
    cur_w: usize,
    has_content: bool,
    rest: &'a [span_t],
    rest_w: usize,
    /// Spaces after the last word; written only if another word follows on the same line.
    pending: Vec<span_t>,
    pending_w: usize,
}

impl wrap_state_t<'_> {
    fn flush(&mut self) {
        let mut next = line_t::new(self.rest.to_vec());
        std::mem::swap(&mut next, &mut self.cur);
        self.lines.push(next);
        self.cur_w = self.rest_w;
        self.has_content = false;
        self.pending.clear();
        self.pending_w = 0;
    }

    fn put_pending(&mut self) {
        for s in std::mem::take(&mut self.pending) {
            self.cur.push(s.text, s.style);
        }
        self.cur_w += self.pending_w;
        self.pending_w = 0;
    }
}

/// Greedy word wrap of styled spans into lines of at most `width` columns. `first` prefixes the
/// first line and `rest` every following line (list bullets, quote rails). A word longer than
/// a line is broken by cluster. Every line makes progress, so a prefix wider than `width` still
/// terminates. `\n` forces a break.
pub fn wrap_spans(
    spans: &[span_t],
    width: usize,
    first: &[span_t],
    rest: &[span_t],
) -> Vec<line_t> {
    let width = width.max(1);
    let mut st = wrap_state_t {
        lines: Vec::new(),
        cur: line_t::new(first.to_vec()),
        cur_w: spans_width(first),
        has_content: false,
        rest,
        rest_w: spans_width(rest),
        pending: Vec::new(),
        pending_w: 0,
    };
    let prefix_rest = st.rest_w;
    for span in spans {
        let text = span.text.as_str();
        let mut pieces = split_pieces(text);
        for piece in pieces.drain(..) {
            match piece {
                piece_t::Newline => st.flush(),
                piece_t::Space(s) => {
                    if !st.has_content {
                        continue;
                    }
                    st.pending.push(span_t::new(s, span.style));
                    st.pending_w += str_width(s);
                }
                piece_t::Word(s) => {
                    let w = str_width(s);
                    if st.cur_w + st.pending_w + w <= width {
                        st.put_pending();
                        st.cur.push(s, span.style);
                        st.cur_w += w;
                        st.has_content = true;
                        continue;
                    }
                    if st.has_content && prefix_rest + w <= width {
                        st.flush();
                        st.cur.push(s, span.style);
                        st.cur_w += w;
                        st.has_content = true;
                        continue;
                    }
                    if st.has_content && st.cur_w + st.pending_w < width {
                        st.put_pending();
                    } else {
                        st.pending.clear();
                        st.pending_w = 0;
                    }
                    for c in clusters(s) {
                        if st.has_content && st.cur_w + c.width > width {
                            st.flush();
                        }
                        st.cur.push(s.get(c.start..c.end).unwrap_or(""), span.style);
                        st.cur_w += c.width;
                        st.has_content = true;
                    }
                }
            }
        }
    }
    if st.has_content || st.lines.is_empty() || st.cur.width() > 0 {
        st.lines.push(st.cur);
    }
    st.lines
}

enum piece_t<'a> {
    Word(&'a str),
    Space(&'a str),
    Newline,
}

fn split_pieces(text: &str) -> Vec<piece_t<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut kind: Option<bool> = None; // Some(true) = in space run
    for (i, c) in text.char_indices() {
        if c == '\n' {
            if let Some(space) = kind.take() {
                out.push(make_piece(text, start, i, space));
            }
            out.push(piece_t::Newline);
            start = i + 1;
            continue;
        }
        let is_space = c == ' ';
        match kind {
            Some(k) if k == is_space => {}
            Some(k) => {
                out.push(make_piece(text, start, i, k));
                start = i;
                kind = Some(is_space);
            }
            None => {
                start = i;
                kind = Some(is_space);
            }
        }
    }
    if let Some(k) = kind {
        out.push(make_piece(text, start, text.len(), k));
    }
    out
}

fn make_piece(text: &str, start: usize, end: usize, space: bool) -> piece_t<'_> {
    let s = text.get(start..end).unwrap_or("");
    if space {
        piece_t::Space(s)
    } else {
        piece_t::Word(s)
    }
}

/// Break text into rows of at most `width` columns by cluster, keeping every space. For code
/// and tool output, where indentation matters. `\n` starts a new row.
pub fn hard_wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = vec![String::new()];
    let mut col = 0usize;
    for c in clusters(text) {
        let s = text.get(c.start..c.end).unwrap_or("");
        if s == "\n" {
            rows.push(String::new());
            col = 0;
            continue;
        }
        if c.width == 0 && s.chars().all(char::is_control) {
            continue;
        }
        if col + c.width > width && col > 0 {
            rows.push(String::new());
            col = 0;
        }
        if let Some(last) = rows.last_mut() {
            last.push_str(s);
        }
        col += c.width;
    }
    rows
}

/// Wrap plain text in one style.
pub fn wrap_plain(text: &str, width: usize, style: Style) -> Vec<line_t> {
    wrap_spans(&[span_t::new(text, style)], width, &[], &[])
}

/// "1.2 s", "340 ms": a measured duration for labels.
pub fn format_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{}.{} s", ms / 1000, (ms % 1000) / 100)
    } else {
        format!("{}m {:02}s", ms / 60_000, (ms % 60_000) / 1000)
    }
}

/// "now", "42s", "5m", "3h", "2d": the age of a timestamp for the sidebar.
pub fn format_age(now_ms: u64, then_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    if secs < 10 {
        "now".to_string()
    } else if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// "1,204": thousands separators for token counts.
pub fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[line_t]) -> Vec<String> {
        lines.iter().map(|l| l.text()).collect()
    }

    #[test]
    fn sanitize_strips_escapes_and_controls() {
        assert_eq!(sanitize("a\u{1b}[31mred\u{1b}[0m b", true), "ared b");
        assert_eq!(sanitize("x\u{1b}]52;c;AAAA\u{7}y", true), "xy");
        assert_eq!(sanitize("x\u{1b}]0;title\u{1b}\\y", true), "xy");
        assert_eq!(sanitize("a\tb", true), "a    b");
        assert_eq!(sanitize("a\r\nb\rc", true), "a\nb\nc");
        assert_eq!(sanitize("a\nb", false), "a b");
        assert_eq!(sanitize("bell\u{7}\u{0}", true), "bell");
        assert_eq!(sanitize("\u{202e}evil", true), "evil");
        assert_eq!(sanitize("trailing\u{1b}", true), "trailing");
    }

    #[test]
    fn widths_of_wide_and_combining() {
        assert_eq!(str_width("abc"), 3);
        assert_eq!(str_width("日本"), 4);
        assert_eq!(str_width("e\u{301}"), 1);
        assert_eq!(clusters("e\u{301}x").len(), 2);
        assert_eq!(str_width("🙂"), 2);
        // A ZWJ family stays one cluster of width 2.
        let family = "👨\u{200d}👩\u{200d}👧";
        assert_eq!(clusters(family).len(), 1);
        assert_eq!(str_width(family), 2);
    }

    #[test]
    fn wrap_basic_and_long_words() {
        let lines = wrap_plain("the quick brown fox jumps", 10, Style::new());
        assert_eq!(texts(&lines), vec!["the quick", "brown fox", "jumps"]);
        let lines = wrap_plain("abcdefghijklmnop", 5, Style::new());
        assert_eq!(texts(&lines), vec!["abcde", "fghij", "klmno", "p"]);
        for l in &lines {
            assert!(l.width() <= 5);
        }
    }

    #[test]
    fn wrap_wide_chars_never_overflow() {
        let lines = wrap_plain("日本語のテキストです", 5, Style::new());
        for l in &lines {
            assert!(l.width() <= 5, "{:?}", l.text());
        }
        assert_eq!(
            lines.iter().map(|l| l.text()).collect::<String>(),
            "日本語のテキストです"
        );
    }

    #[test]
    fn wrap_with_prefixes() {
        let first = [span_t::new("• ", Style::new())];
        let rest = [span_t::new("  ", Style::new())];
        let lines = wrap_spans(
            &[span_t::new("one two three four", Style::new())],
            9,
            &first,
            &rest,
        );
        assert_eq!(texts(&lines), vec!["• one two", "  three", "  four"]);
    }

    #[test]
    fn wrap_terminates_with_huge_prefix_and_zero_width() {
        let first = [span_t::new("##########", Style::new())];
        let lines = wrap_spans(&[span_t::new("abc", Style::new())], 3, &first, &first);
        assert!(!lines.is_empty());
        let lines = wrap_plain("abc", 0, Style::new());
        assert_eq!(texts(&lines), vec!["a", "b", "c"]);
    }

    #[test]
    fn wrap_newlines_and_empty() {
        assert_eq!(texts(&wrap_plain("a\nb", 10, Style::new())), vec!["a", "b"]);
        assert_eq!(texts(&wrap_plain("", 10, Style::new())), vec![""]);
        assert_eq!(
            texts(&wrap_plain("a\n\nb", 10, Style::new())),
            vec!["a", "", "b"]
        );
    }

    #[test]
    fn wrap_huge_line_is_fast_and_bounded() {
        let big = "word ".repeat(40_000);
        let start = std::time::Instant::now();
        let lines = wrap_plain(&big, 80, Style::new());
        assert!(lines.iter().all(|l| l.width() <= 80));
        assert!(start.elapsed().as_secs() < 5);
    }

    #[test]
    fn hard_wrap_keeps_indentation() {
        assert_eq!(hard_wrap("    x = 1", 6), vec!["    x ", "= 1"]);
        assert_eq!(hard_wrap("a\n\n  b", 10), vec!["a", "", "  b"]);
        assert_eq!(hard_wrap("日本語", 3), vec!["日", "本", "語"]);
        assert_eq!(hard_wrap("", 3), vec![""]);
    }

    #[test]
    fn truncate_and_fit() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(truncate("日本語", 3), "日…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(fit("ab", 4), "ab  ");
    }

    #[test]
    fn formats() {
        assert_eq!(format_ms(340), "340 ms");
        assert_eq!(format_ms(1250), "1.2 s");
        assert_eq!(format_ms(61_000), "1m 01s");
        assert_eq!(format_age(100_000, 95_000), "now");
        assert_eq!(format_age(100_000, 40_000), "1m");
        assert_eq!(format_age(10_000, 20_000), "now");
        assert_eq!(group_digits(1204), "1,204");
        assert_eq!(group_digits(12), "12");
        assert_eq!(group_digits(1_000_000), "1,000,000");
    }
}
