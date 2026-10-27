/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The multi-line message editor. The cursor is a byte offset that always sits on a cluster
//! boundary, so editing never splits a combining mark or a wide character.

use crate::text::{clusters, sanitize, str_width};

/// Largest message the composer keeps, in bytes. A longer paste is cut.
pub const MaxInput: usize = 64 * 1024;
const MaxHistory: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditOp {
    Insert(char),
    Paste(String),
    Newline,
    Backspace,
    Delete,
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    Up,
    Down,
    KillToStart,
    KillToEnd,
    KillWordBack,
    Clear,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct composer_t {
    text: String,
    cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    stash: String,
    /// Selected row of the slash-command popup.
    pub popup_index: usize,
}

impl composer_t {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
        self.popup_index = 0;
    }

    /// Take the text for sending and remember it in history.
    pub fn take(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.history_pos = None;
        self.stash.clear();
        self.popup_index = 0;
        if !text.trim().is_empty() && self.history.last() != Some(&text) {
            self.history.push(text.clone());
            if self.history.len() > MaxHistory {
                self.history.remove(0);
            }
        }
        text
    }

    fn boundaries(&self) -> Vec<usize> {
        let mut b: Vec<usize> = clusters(&self.text).iter().map(|c| c.start).collect();
        b.push(self.text.len());
        b
    }

    fn prev_boundary(&self) -> usize {
        self.boundaries()
            .into_iter()
            .rev()
            .find(|b| *b < self.cursor)
            .unwrap_or(0)
    }

    fn next_boundary(&self) -> usize {
        self.boundaries()
            .into_iter()
            .find(|b| *b > self.cursor)
            .unwrap_or(self.text.len())
    }

    fn insert_str(&mut self, s: &str) {
        let room = MaxInput.saturating_sub(self.text.len());
        let mut cut = s.len().min(room);
        while cut > 0 && !s.is_char_boundary(cut) {
            cut -= 1;
        }
        let piece = s.get(..cut).unwrap_or("");
        if piece.is_empty() {
            return;
        }
        self.text.insert_str(self.cursor, piece);
        self.cursor += piece.len();
        self.popup_index = 0;
    }

    fn line_start(&self) -> usize {
        self.text
            .get(..self.cursor)
            .and_then(|s| s.rfind('\n'))
            .map(|i| i + 1)
            .unwrap_or(0)
    }

    fn line_end(&self) -> usize {
        self.text
            .get(self.cursor..)
            .and_then(|s| s.find('\n'))
            .map(|i| self.cursor + i)
            .unwrap_or(self.text.len())
    }

    fn word_left(&self) -> usize {
        let before = self.text.get(..self.cursor).unwrap_or("");
        let trimmed = before.trim_end_matches(char::is_whitespace);
        trimmed
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0)
    }

    fn word_right(&self) -> usize {
        let after = self.text.get(self.cursor..).unwrap_or("");
        let skip_ws = after.len() - after.trim_start_matches(char::is_whitespace).len();
        let rest = after.get(skip_ws..).unwrap_or("");
        let word = rest.find(char::is_whitespace).unwrap_or(rest.len());
        self.cursor + skip_ws + word
    }

    /// Column (in clusters' widths) of the cursor within its logical line.
    fn column(&self) -> usize {
        str_width(self.text.get(self.line_start()..self.cursor).unwrap_or(""))
    }

    fn offset_at_column(&self, line_start: usize, line_end: usize, column: usize) -> usize {
        let line = self.text.get(line_start..line_end).unwrap_or("");
        let mut used = 0usize;
        for c in clusters(line) {
            if used + c.width > column {
                return line_start + c.start;
            }
            used += c.width;
        }
        line_end
    }

    /// Apply an edit. Returns false when Up or Down had no line to move to, so the caller can
    /// treat it as history navigation.
    pub fn apply(&mut self, op: EditOp) -> bool {
        match op {
            EditOp::Insert(c) => {
                if !c.is_control() {
                    let mut buf = [0u8; 4];
                    self.insert_str(c.encode_utf8(&mut buf));
                }
            }
            EditOp::Paste(s) => {
                let clean = sanitize(&s, true);
                self.insert_str(&clean);
            }
            EditOp::Newline => self.insert_str("\n"),
            EditOp::Backspace => {
                let start = self.prev_boundary();
                if start < self.cursor {
                    self.text.replace_range(start..self.cursor, "");
                    self.cursor = start;
                }
            }
            EditOp::Delete => {
                let end = self.next_boundary();
                if end > self.cursor {
                    self.text.replace_range(self.cursor..end, "");
                }
            }
            EditOp::Left => self.cursor = self.prev_boundary(),
            EditOp::Right => self.cursor = self.next_boundary(),
            EditOp::WordLeft => self.cursor = self.word_left(),
            EditOp::WordRight => self.cursor = self.word_right(),
            EditOp::LineStart => self.cursor = self.line_start(),
            EditOp::LineEnd => self.cursor = self.line_end(),
            EditOp::Up => {
                let start = self.line_start();
                if start == 0 {
                    return self.history_prev();
                }
                let col = self.column();
                let prev_end = start - 1;
                let prev_start = self
                    .text
                    .get(..prev_end)
                    .and_then(|s| s.rfind('\n'))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                self.cursor = self.offset_at_column(prev_start, prev_end, col);
            }
            EditOp::Down => {
                let end = self.line_end();
                if end >= self.text.len() {
                    return self.history_next();
                }
                let col = self.column();
                let next_start = end + 1;
                let next_end = self
                    .text
                    .get(next_start..)
                    .and_then(|s| s.find('\n'))
                    .map(|i| next_start + i)
                    .unwrap_or(self.text.len());
                self.cursor = self.offset_at_column(next_start, next_end, col);
            }
            EditOp::KillToStart => {
                let start = self.line_start();
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            EditOp::KillToEnd => {
                let end = self.line_end();
                let end = if end == self.cursor && end < self.text.len() {
                    end + 1
                } else {
                    end
                };
                self.text.replace_range(self.cursor..end, "");
            }
            EditOp::KillWordBack => {
                let start = self.word_left();
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
            }
            EditOp::Clear => {
                self.text.clear();
                self.cursor = 0;
            }
        }
        true
    }

    fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let pos = match self.history_pos {
            None => {
                self.stash = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => return false,
            Some(p) => p - 1,
        };
        self.history_pos = Some(pos);
        if let Some(h) = self.history.get(pos) {
            self.text = h.clone();
            self.cursor = self.text.len();
        }
        true
    }

    fn history_next(&mut self) -> bool {
        let Some(pos) = self.history_pos else {
            return false;
        };
        if pos + 1 >= self.history.len() {
            self.history_pos = None;
            self.text = std::mem::take(&mut self.stash);
        } else {
            self.history_pos = Some(pos + 1);
            self.text = self.history.get(pos + 1).cloned().unwrap_or_default();
        }
        self.cursor = self.text.len();
        true
    }

    /// The text broken into display rows of at most `width` columns, with the cursor's row
    /// and column. Rows break at newlines and, inside a line, at the width.
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut rows = vec![String::new()];
        let mut col = 0usize;
        let mut cursor = (0usize, 0usize);
        let mut found = false;
        for c in clusters(&self.text) {
            let here = !found && c.start >= self.cursor;
            if here {
                cursor = (rows.len() - 1, col);
                found = true;
            }
            let s = self.text.get(c.start..c.end).unwrap_or("");
            if s == "\n" {
                rows.push(String::new());
                col = 0;
                continue;
            }
            if col + c.width > width {
                rows.push(String::new());
                col = 0;
                if here {
                    cursor = (rows.len() - 1, 0);
                }
            }
            if let Some(last) = rows.last_mut() {
                last.push_str(s);
            }
            col += c.width;
        }
        if !found {
            if col >= width {
                rows.push(String::new());
                col = 0;
            }
            cursor = (rows.len() - 1, col);
        }
        (rows, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> composer_t {
        let mut c = composer_t::new();
        for ch in s.chars() {
            if ch == '\n' {
                c.apply(EditOp::Newline);
            } else {
                c.apply(EditOp::Insert(ch));
            }
        }
        c
    }

    #[test]
    fn insert_and_backspace_by_cluster() {
        let mut c = typed("ae\u{301}");
        assert_eq!(c.text(), "ae\u{301}");
        c.apply(EditOp::Backspace);
        assert_eq!(c.text(), "a");
        let mut c = typed("日本");
        c.apply(EditOp::Left);
        c.apply(EditOp::Delete);
        assert_eq!(c.text(), "日");
        c.apply(EditOp::Backspace);
        c.apply(EditOp::Backspace);
        assert_eq!(c.text(), "");
    }

    #[test]
    fn words_and_lines() {
        let mut c = typed("hello big world");
        c.apply(EditOp::WordLeft);
        assert_eq!(c.cursor(), 10);
        c.apply(EditOp::WordLeft);
        assert_eq!(c.cursor(), 6);
        c.apply(EditOp::WordRight);
        assert_eq!(c.cursor(), 9);
        c.apply(EditOp::LineStart);
        assert_eq!(c.cursor(), 0);
        c.apply(EditOp::KillToEnd);
        assert_eq!(c.text(), "");
        let mut c = typed("one two");
        c.apply(EditOp::KillWordBack);
        assert_eq!(c.text(), "one ");
        c.apply(EditOp::KillToStart);
        assert_eq!(c.text(), "");
    }

    #[test]
    fn multi_line_up_down_keeps_column() {
        let mut c = typed("abcd\nxy\nlong line");
        c.apply(EditOp::Up);
        assert_eq!(c.cursor(), 7); // end of "xy"
        c.apply(EditOp::Up);
        assert_eq!(c.cursor(), 2);
        c.apply(EditOp::Down);
        assert_eq!(c.cursor(), 7);
        c.apply(EditOp::LineEnd);
        c.apply(EditOp::Down);
        assert_eq!(c.cursor(), 10);
    }

    #[test]
    fn history_round_trip() {
        let mut c = typed("first");
        c.take();
        let mut c2 = c.clone();
        for ch in "second".chars() {
            c2.apply(EditOp::Insert(ch));
        }
        c2.take();
        c2.apply(EditOp::Insert('x'));
        assert!(c2.apply(EditOp::Up));
        assert_eq!(c2.text(), "second");
        assert!(c2.apply(EditOp::Up));
        assert_eq!(c2.text(), "first");
        assert!(!c2.apply(EditOp::Up));
        assert!(c2.apply(EditOp::Down));
        assert_eq!(c2.text(), "second");
        assert!(c2.apply(EditOp::Down));
        assert_eq!(c2.text(), "x");
        assert!(!c2.apply(EditOp::Down));
    }

    #[test]
    fn paste_is_sanitized_and_capped() {
        let mut c = composer_t::new();
        c.apply(EditOp::Paste("a\r\nb\u{1b}[31m\tc".into()));
        assert_eq!(c.text(), "a\nb    c");
        let mut c = composer_t::new();
        c.apply(EditOp::Paste("é".repeat(MaxInput)));
        assert!(c.text().len() <= MaxInput);
        c.apply(EditOp::Insert('z'));
        assert!(c.text().len() <= MaxInput);
    }

    #[test]
    fn control_chars_are_not_inserted() {
        let mut c = composer_t::new();
        c.apply(EditOp::Insert('\u{7}'));
        assert!(c.is_empty());
    }

    #[test]
    fn layout_wraps_and_places_cursor() {
        let c = typed("abcdef");
        let (rows, cursor) = c.layout(4);
        assert_eq!(rows, vec!["abcd", "ef"]);
        assert_eq!(cursor, (1, 2));
        let c = typed("abcd");
        let (rows, cursor) = c.layout(4);
        assert_eq!(rows, vec!["abcd", ""]);
        assert_eq!(cursor, (1, 0));
        let mut c = typed("ab\ncd");
        c.apply(EditOp::Up);
        let (rows, cursor) = c.layout(10);
        assert_eq!(rows, vec!["ab", "cd"]);
        assert_eq!(cursor, (0, 2));
        let c = typed("日本語");
        let (rows, _) = c.layout(3);
        assert_eq!(rows, vec!["日", "本", "語"]);
    }

    #[test]
    fn take_records_history_and_clears() {
        let mut c = typed("  ");
        assert_eq!(c.take(), "  ");
        assert!(c.is_empty());
        assert!(!c.apply(EditOp::Up));
    }
}
