/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Markdown to styled, wrapped lines through `pulldown-cmark` (D49). No box characters: quotes
//! are a rail, code is an inset tone, tables are aligned columns with a toned header, and a rule
//! is a tone strip. Input is untrusted, so every text run is sanitized and nothing panics.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};

use crate::text::{
    fit, hard_wrap, line_t, sanitize, span_t, spans_width, str_width, truncate, wrap_spans,
};
use crate::theme::theme_t;

/// Deeper containers still render, but only this many draw a prefix.
const MaxPrefixDepth: usize = 8;

enum container_t {
    Quote,
    List { next: Option<u64>, depth: usize },
    Item { marker: String, used: bool },
}

struct table_t {
    aligns: Vec<Alignment>,
    rows: Vec<(bool, Vec<Vec<span_t>>)>,
    row: Vec<Vec<span_t>>,
    in_head: bool,
}

struct md_t<'t> {
    theme: &'t theme_t,
    width: usize,
    out: Vec<line_t>,
    inline: Vec<span_t>,
    styles: Vec<Style>,
    containers: Vec<container_t>,
    link_urls: Vec<String>,
    code: Option<(String, String)>,
    table: Option<table_t>,
    cell: Option<Vec<span_t>>,
    pending_gap: bool,
    task: Option<bool>,
}

/// Render Markdown to lines no wider than `width` (prefixes included).
pub fn render(text: &str, width: usize, theme: &theme_t) -> Vec<line_t> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut md = md_t {
        theme,
        width: width.max(1),
        out: Vec::new(),
        inline: Vec::new(),
        styles: vec![Style::new().fg(theme.fg)],
        containers: Vec::new(),
        link_urls: Vec::new(),
        code: None,
        table: None,
        cell: None,
        pending_gap: false,
        task: None,
    };
    for event in Parser::new_ext(text, options) {
        md.event(event);
    }
    md.flush_inline();
    if md.out.is_empty() {
        md.out.push(line_t::blank());
    }
    md.out
}

impl md_t<'_> {
    fn style(&self) -> Style {
        self.styles.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, f: impl FnOnce(Style) -> Style) {
        let next = f(self.style());
        self.styles.push(next);
    }

    fn pop_style(&mut self) {
        if self.styles.len() > 1 {
            self.styles.pop();
        }
    }

    fn text(&mut self, raw: &str) {
        let clean = sanitize(raw, true);
        if clean.is_empty() {
            return;
        }
        let style = self.style();
        if let Some(code) = self.code.as_mut() {
            code.1.push_str(&clean);
            return;
        }
        let span = span_t::new(clean, style);
        match self.cell.as_mut() {
            Some(cell) => cell.push(span),
            None => self.inline.push(span),
        }
    }

    fn prefixes(&self) -> (Vec<span_t>, Vec<span_t>) {
        let rail = Style::new().fg(self.theme.fg_faint);
        let bullet = Style::new().fg(self.theme.agent);
        let mut first = Vec::new();
        let mut rest = Vec::new();
        for container in self.containers.iter().take(MaxPrefixDepth) {
            match container {
                container_t::Quote => {
                    first.push(span_t::new("▎ ", rail));
                    rest.push(span_t::new("▎ ", rail));
                }
                container_t::Item { marker, used } => {
                    let pad = " ".repeat(str_width(marker));
                    if *used {
                        first.push(span_t::new(pad.clone(), Style::new()));
                    } else {
                        first.push(span_t::new(marker.clone(), bullet));
                    }
                    rest.push(span_t::new(pad, Style::new()));
                }
                container_t::List { .. } => {}
            }
        }
        (first, rest)
    }

    fn mark_items_used(&mut self) {
        for container in self.containers.iter_mut() {
            if let container_t::Item { used, .. } = container {
                *used = true;
            }
        }
    }

    fn gap(&mut self) {
        if self.pending_gap && !self.out.is_empty() {
            let (_, rest) = self.prefixes();
            let mut line = line_t::new(rest);
            // Trailing spaces of a list pad carry no meaning on a blank line.
            line.spans.retain(|s| s.text.trim() != "");
            self.out.push(line);
        }
        self.pending_gap = false;
    }

    fn flush_inline(&mut self) {
        if self.inline.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.inline);
        self.gap();
        let (mut first, rest) = self.prefixes();
        if let Some(done) = self.task.take() {
            let (glyph, style) = if done {
                ("✓ ", Style::new().fg(self.theme.success))
            } else {
                ("○ ", Style::new().fg(self.theme.fg_dim))
            };
            first.push(span_t::new(glyph, style));
        }
        let lines = wrap_spans(&spans, self.width, &first, &rest);
        self.out.extend(lines);
        self.mark_items_used();
        self.pending_gap = true;
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                let style = self
                    .style()
                    .fg(self.theme.code)
                    .bg(self.theme.inset)
                    .remove_modifier(Modifier::UNDERLINED);
                let clean = sanitize(&t, false);
                let span = span_t::new(format!(" {clean} "), style);
                match self.cell.as_mut() {
                    Some(cell) => cell.push(span),
                    None => self.inline.push(span),
                }
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => self.text(&t),
            Event::Html(t) | Event::InlineHtml(t) => {
                let dim = self.theme.fg_dim;
                self.push_style(|s| s.fg(dim));
                self.text(&t);
                self.pop_style();
            }
            Event::FootnoteReference(t) => self.text(&format!("[{t}]")),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => {
                if self.code.is_none() {
                    let style = self.style();
                    self.inline.push(span_t::new("\n", style));
                }
            }
            Event::Rule => {
                self.flush_inline();
                self.gap();
                let (first, _) = self.prefixes();
                let pad = self.width.saturating_sub(spans_width(&first));
                let mut line = line_t::new(first);
                line.push(" ".repeat(pad), Style::new().bg(self.theme.panel));
                self.out.push(line);
                self.pending_gap = true;
            }
            Event::TaskListMarker(done) => self.task = Some(done),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.flush_inline(),
            Tag::Heading { level, .. } => {
                self.flush_inline();
                let theme = self.theme;
                self.push_style(|s| match level {
                    HeadingLevel::H1 => s.fg(theme.ice).add_modifier(Modifier::BOLD),
                    HeadingLevel::H2 => s.fg(theme.fg).add_modifier(Modifier::BOLD),
                    HeadingLevel::H3 => s.fg(theme.fg_dim).add_modifier(Modifier::BOLD),
                    _ => s.fg(theme.fg_dim).add_modifier(Modifier::ITALIC),
                });
                if level == HeadingLevel::H1 {
                    let style = Style::new().fg(theme.agent);
                    self.inline.push(span_t::new("◆ ", style));
                }
            }
            Tag::BlockQuote(_) => {
                self.flush_inline();
                self.containers.push(container_t::Quote);
                let dim = self.theme.fg_dim;
                self.push_style(|s| s.fg(dim).add_modifier(Modifier::ITALIC));
            }
            Tag::CodeBlock(kind) => {
                self.flush_inline();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => sanitize(&info, false)
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => self.flush_inline(),
            Tag::List(start) => {
                self.flush_inline();
                let depth = self
                    .containers
                    .iter()
                    .filter(|c| matches!(c, container_t::List { .. }))
                    .count();
                if depth == 0 {
                    // A top-level list is its own block.
                    self.gap();
                } else {
                    self.pending_gap = false;
                }
                self.containers
                    .push(container_t::List { next: start, depth });
            }
            Tag::Item => {
                self.flush_inline();
                self.pending_gap = false;
                let marker = match self.containers.last_mut() {
                    Some(container_t::List { next: Some(n), .. }) => {
                        let m = format!("{n}. ");
                        *n = n.saturating_add(1);
                        m
                    }
                    Some(container_t::List { depth, .. }) => match *depth % 3 {
                        0 => "• ".to_string(),
                        1 => "◦ ".to_string(),
                        _ => "▪ ".to_string(),
                    },
                    _ => "• ".to_string(),
                };
                self.containers.push(container_t::Item {
                    marker,
                    used: false,
                });
            }
            Tag::Table(aligns) => {
                self.flush_inline();
                self.table = Some(table_t {
                    aligns,
                    rows: Vec::new(),
                    row: Vec::new(),
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = true;
                    t.row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.row.clear();
                }
            }
            Tag::TableCell => self.cell = Some(Vec::new()),
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Superscript | Tag::Subscript => self.push_style(|s| s),
            Tag::Link { dest_url, .. } => {
                self.link_urls.push(sanitize(&dest_url, false));
                let link = self.theme.link;
                self.push_style(|s| s.fg(link).add_modifier(Modifier::UNDERLINED));
            }
            Tag::Image { dest_url, .. } => {
                self.link_urls.push(sanitize(&dest_url, false));
                let dim = self.theme.fg_dim;
                self.push_style(|s| s.fg(dim));
                self.text("[image: ");
            }
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => self.flush_inline(),
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_inline(),
            TagEnd::Heading(_) => {
                self.flush_inline();
                self.pop_style();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                self.pop_style();
                if let Some(pos) = self
                    .containers
                    .iter()
                    .rposition(|c| matches!(c, container_t::Quote))
                {
                    self.containers.truncate(pos);
                }
                self.pending_gap = true;
            }
            TagEnd::CodeBlock => self.end_code(),
            TagEnd::HtmlBlock => self.flush_inline(),
            TagEnd::List(_) => {
                self.flush_inline();
                if let Some(pos) = self
                    .containers
                    .iter()
                    .rposition(|c| matches!(c, container_t::List { .. }))
                {
                    self.containers.truncate(pos);
                }
                self.pending_gap = true;
            }
            TagEnd::Item => {
                self.flush_inline();
                self.task = None;
                if let Some(pos) = self
                    .containers
                    .iter()
                    .rposition(|c| matches!(c, container_t::Item { .. }))
                {
                    self.containers.truncate(pos);
                }
                self.pending_gap = false;
            }
            TagEnd::Table => self.end_table(),
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push((true, row));
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push((false, row));
                }
            }
            TagEnd::TableCell => {
                let cell = self.cell.take().unwrap_or_default();
                if let Some(t) = self.table.as_mut() {
                    t.row.push(cell);
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                let url = self.link_urls.pop().unwrap_or_default();
                let label: String = self
                    .inline
                    .last()
                    .map(|s| s.text.clone())
                    .unwrap_or_default();
                if !url.is_empty() && label.trim() != url.trim() {
                    let style = Style::new().fg(self.theme.fg_faint);
                    self.text_styled(&format!(" ({url})"), style);
                }
            }
            TagEnd::Image => {
                let url = self.link_urls.pop().unwrap_or_default();
                self.text("]");
                self.pop_style();
                if !url.is_empty() {
                    let style = Style::new().fg(self.theme.fg_faint);
                    self.text_styled(&format!(" ({url})"), style);
                }
            }
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => self.flush_inline(),
        }
    }

    fn text_styled(&mut self, text: &str, style: Style) {
        let span = span_t::new(sanitize(text, false), style);
        match self.cell.as_mut() {
            Some(cell) => cell.push(span),
            None => self.inline.push(span),
        }
    }

    fn end_code(&mut self) {
        let Some((lang, body)) = self.code.take() else {
            return;
        };
        self.gap();
        let theme = self.theme;
        let (first, rest) = self.prefixes();
        let prefix_w = spans_width(&first).max(spans_width(&rest));
        let inner = self.width.saturating_sub(prefix_w).max(1);
        let code_style = Style::new().fg(theme.fg).bg(theme.inset);
        let label_style = Style::new()
            .fg(theme.fg_faint)
            .bg(theme.inset)
            .add_modifier(Modifier::ITALIC);
        let mut first_line = true;
        let mut emit = |md: &mut Self, content: Vec<span_t>| {
            let prefix = if first_line {
                first.clone()
            } else {
                rest.clone()
            };
            first_line = false;
            let mut line = line_t::new(prefix);
            let used: usize = spans_width(&content);
            for s in content {
                line.push(s.text, s.style);
            }
            line.push(" ".repeat(inner.saturating_sub(used)), code_style);
            md.out.push(line);
        };
        if !lang.is_empty() {
            let label = fit(
                &format!(" {}", truncate(&lang, inner.saturating_sub(1))),
                inner,
            );
            emit(self, vec![span_t::new(label, label_style)]);
        }
        let body = body.strip_suffix('\n').unwrap_or(&body).to_string();
        for row in hard_wrap(&body, inner.saturating_sub(1).max(1)) {
            emit(self, vec![span_t::new(format!(" {row}"), code_style)]);
        }
        self.mark_items_used();
        self.pending_gap = true;
    }

    fn end_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        self.gap();
        let theme = self.theme;
        let (first, rest) = self.prefixes();
        let avail = self
            .width
            .saturating_sub(spans_width(&first).max(spans_width(&rest)))
            .max(1);
        let ncols = table.rows.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
        if ncols == 0 {
            return;
        }
        let mut widths = vec![1usize; ncols];
        for (_, row) in &table.rows {
            for (i, cell) in row.iter().enumerate() {
                if let Some(w) = widths.get_mut(i) {
                    *w = (*w).max(spans_width(cell));
                }
            }
        }
        let gap = 2usize;
        let gaps = gap * ncols.saturating_sub(1);
        let total: usize = widths.iter().sum::<usize>() + gaps;
        if total > avail {
            let target = avail.saturating_sub(gaps).max(ncols);
            let sum: usize = widths.iter().sum::<usize>().max(1);
            for w in widths.iter_mut() {
                *w = ((*w * target) / sum).max(1);
            }
        }
        let mut first_line = true;
        for (index, (head, row)) in table.rows.iter().enumerate() {
            let bg = if *head {
                Some(theme.inset)
            } else if index % 2 == 0 {
                Some(theme.side)
            } else {
                None
            };
            let prefix = if first_line {
                first.clone()
            } else {
                rest.clone()
            };
            first_line = false;
            let mut line = line_t::new(prefix);
            for (i, w) in widths.iter().enumerate() {
                let cell_text: String = row
                    .get(i)
                    .map(|c| c.iter().map(|s| s.text.as_str()).collect())
                    .unwrap_or_default();
                let cell_style = row
                    .get(i)
                    .and_then(|c| c.first())
                    .map(|s| s.style)
                    .unwrap_or_else(|| Style::new().fg(theme.fg));
                let mut style = if *head {
                    cell_style.add_modifier(Modifier::BOLD)
                } else {
                    cell_style
                };
                if let Some(bg) = bg {
                    style = style.bg(bg);
                }
                let cut = truncate(cell_text.trim(), *w);
                let pad = w.saturating_sub(str_width(&cut));
                let align = table.aligns.get(i).copied().unwrap_or(Alignment::None);
                let (left, right) = match align {
                    Alignment::Right => (pad, 0),
                    Alignment::Center => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                let pad_style = bg.map(|b| Style::new().bg(b)).unwrap_or_default();
                line.push(" ".repeat(left), pad_style);
                line.push(cut, style);
                line.push(" ".repeat(right), pad_style);
                if i + 1 < ncols {
                    line.push(" ".repeat(gap), pad_style);
                }
            }
            self.out.push(line);
        }
        self.mark_items_used();
        self.pending_gap = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, width: usize) -> Vec<String> {
        render(text, width, &theme_t::dark())
            .iter()
            .map(|l| l.text())
            .collect()
    }

    #[test]
    fn paragraphs_and_emphasis() {
        let out = render("Hello *there* **bold** ~~gone~~", 80, &theme_t::dark());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text(), "Hello there bold gone");
        let styles: Vec<_> = out[0].spans.iter().map(|s| s.style).collect();
        assert!(
            styles
                .iter()
                .any(|s| s.add_modifier.contains(Modifier::ITALIC))
        );
        assert!(
            styles
                .iter()
                .any(|s| s.add_modifier.contains(Modifier::BOLD))
        );
        assert!(
            styles
                .iter()
                .any(|s| s.add_modifier.contains(Modifier::CROSSED_OUT))
        );
        assert_eq!(lines("one\n\ntwo", 80), vec!["one", "", "two"]);
    }

    #[test]
    fn headings() {
        let out = lines("# Title\n\n## Sub\n\ntext", 80);
        assert_eq!(out, vec!["◆ Title", "", "Sub", "", "text"]);
    }

    #[test]
    fn inline_code_and_links() {
        let out = lines("use `x` and [docs](https://e.org)", 80);
        assert_eq!(out, vec!["use  x  and docs (https://e.org)"]);
        let out = lines("<https://e.org>", 80);
        assert_eq!(out, vec!["https://e.org"]);
    }

    #[test]
    fn fenced_code_with_label() {
        let out = render("```rust\nfn main() {}\n```", 20, &theme_t::dark());
        let text: Vec<String> = out.iter().map(|l| l.text()).collect();
        assert_eq!(text[0].trim_end(), " rust");
        assert_eq!(text[1].trim_end(), " fn main() {}");
        assert!(text.iter().all(|t| str_width(t) == 20));
    }

    #[test]
    fn code_keeps_indentation() {
        let out = lines("```\nfn x() {\n    y();\n}\n```", 30);
        assert_eq!(out[1].trim_end(), "     y();");
    }

    #[test]
    fn nested_and_ordered_lists() {
        let out = lines("- a\n  - b\n    - c\n- d\n\n3. x\n4. y", 40);
        assert_eq!(
            out,
            vec!["• a", "  ◦ b", "    ▪ c", "• d", "", "3. x", "4. y"]
        );
    }

    #[test]
    fn list_item_wraps_under_its_text() {
        let out = lines("- alpha beta gamma delta", 12);
        assert_eq!(out, vec!["• alpha beta", "  gamma", "  delta"]);
    }

    #[test]
    fn task_list() {
        let out = lines("- [ ] todo\n- [x] done", 40);
        assert_eq!(out, vec!["• ○ todo", "• ✓ done"]);
    }

    #[test]
    fn block_quote_rail() {
        let out = lines("> quoted words here", 12);
        assert_eq!(out, vec!["▎ quoted", "▎ words here"]);
    }

    #[test]
    fn rule_is_tone_strip_without_box_chars() {
        let out = render("a\n\n---\n\nb", 10, &theme_t::dark());
        let rule = out
            .iter()
            .find(|l| l.text().trim().is_empty() && l.width() == 10);
        assert!(rule.is_some());
        for l in &out {
            assert!(!l.text().contains('─'));
        }
    }

    #[test]
    fn table_aligned_columns() {
        let md = "| a | bb |\n|---|---:|\n| ccc | d |";
        let out = lines(md, 40);
        assert_eq!(out, vec!["a    bb", "ccc   d"]);
    }

    #[test]
    fn table_shrinks_to_width() {
        let md = "| aaaaaaaaaaaaaaaa | bbbbbbbbbbbbbbbbbbbbb |\n|---|---|\n| x | y |";
        for l in render(md, 20, &theme_t::dark()) {
            assert!(l.width() <= 20, "{:?}", l.text());
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        let theme = theme_t::dark();
        let cases = [
            "",
            "\u{1b}[2J\u{1b}]52;c;AAAA\u{7}",
            &"> ".repeat(500),
            &"- ".repeat(500),
            &"[".repeat(2000),
            &"`".repeat(999),
            "| a |\n|---|\n| b | c | d |",
            "```\nunterminated",
            "e\u{301}\u{301}\u{301} 🙂🙂 日本語 \u{200d}",
            &"x".repeat(100_000),
        ];
        for case in cases {
            for width in [0usize, 1, 2, 7, 40] {
                let out = render(case, width, &theme);
                assert!(!out.is_empty());
                for l in &out {
                    assert!(!l.text().contains('\u{1b}'));
                }
            }
        }
    }

    #[test]
    fn wide_text_wraps_within_width() {
        for l in render(
            "日本語のテキストと 🙂 emoji mixed in a paragraph",
            9,
            &theme_t::dark(),
        ) {
            assert!(l.width() <= 9, "{:?}", l.text());
        }
    }
}
