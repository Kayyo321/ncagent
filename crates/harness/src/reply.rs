/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The reply parser: text, `@call` blocks and `@ask` blocks (D13).
//!
//! The parser never panics and never fails. Each problem becomes an issue string, and a block
//! with an issue yields no item. Untrusted names and values are quoted with `{:?}` and cut
//! short in issues, so a message cannot carry raw control characters into a log or a terminal.

use crate::event::{ReplyItem, question_t};
use crate::tools::{find, parsed_call_t, tool_arg_t, tool_spec_t};

/// Longest untrusted fragment shown in an issue, in characters.
const ShownLimit: usize = 40;

/// The result of parsing one reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct parsed_reply_t {
    pub items: Vec<ReplyItem>,
    pub issues: Vec<String>,
    /// Text after the last block was dropped, because the reply holds a valid call block.
    pub ignored_trailing_text: bool,
}

impl parsed_reply_t {
    /// True when there are no issues and at least one item.
    pub fn valid(&self) -> bool {
        self.issues.is_empty() && !self.items.is_empty()
    }
}

/// Parses a reply into text, call and question items, and lists every issue found.
pub fn parse_reply(text: &str) -> parsed_reply_t {
    let mut parser = reply_parser_t::default();
    for (index, raw) in text.lines().enumerate() {
        parser.line(index + 1, raw);
    }
    parser.finish()
}

/// A line that opens a block.
enum Opener<'a> {
    Call(&'a str),
    Ask,
}

/// A line that is one of the markers of the grammar, after trailing whitespace is removed.
enum Marker<'a> {
    Open(Opener<'a>),
    End,
}

fn marker_of(trimmed: &str) -> Option<Marker<'_>> {
    match trimmed {
        "@end" => Some(Marker::End),
        "@ask" => Some(Marker::Open(Opener::Ask)),
        "@call" => Some(Marker::Open(Opener::Call(""))),
        _ => trimmed
            .strip_prefix("@call ")
            .map(|name| Marker::Open(Opener::Call(name))),
    }
}

/// A block that has been opened and not yet closed.
enum OpenBlock {
    Call {
        line: usize,
        tool: String,
        args: Vec<tool_arg_t>,
        /// The number of issues before this block. A block that adds one yields no item.
        issue_base: usize,
    },
    Ask {
        line: usize,
        question: Option<String>,
        options: Option<Vec<String>>,
        issue_base: usize,
    },
}

impl OpenBlock {
    fn line(&self) -> usize {
        match self {
            OpenBlock::Call { line, .. } | OpenBlock::Ask { line, .. } => *line,
        }
    }
}

#[derive(Default)]
struct reply_parser_t<'a> {
    items: Vec<ReplyItem>,
    issues: Vec<String>,
    /// Text lines seen outside any block, not yet turned into an item.
    pending: Vec<&'a str>,
    block: Option<OpenBlock>,
}

impl<'a> reply_parser_t<'a> {
    fn line(&mut self, number: usize, raw: &'a str) {
        let trimmed = raw.trim_end();
        match (self.block.take(), marker_of(trimmed)) {
            (Some(block), Some(Marker::End)) => self.close(block),
            (Some(block), Some(Marker::Open(opener))) => {
                self.issues.push(format!(
                    "line {number}: a block opens before the @end of the block from line {}",
                    block.line()
                ));
                self.open(number, opener);
            }
            (Some(block), None) => {
                let block = self.add_line(block, number, raw);
                self.block = Some(block);
            }
            (None, Some(Marker::End)) => {
                self.flush_text();
                self.issues
                    .push(format!("line {number}: @end without an open block"));
            }
            (None, Some(Marker::Open(opener))) => self.open(number, opener),
            (None, None) => self.pending.push(raw),
        }
    }

    fn open(&mut self, number: usize, opener: Opener<'_>) {
        self.flush_text();
        let issue_base = self.issues.len();
        self.block = Some(match opener {
            Opener::Call(name) => OpenBlock::Call {
                line: number,
                tool: name.to_string(),
                args: Vec::new(),
                issue_base,
            },
            Opener::Ask => OpenBlock::Ask {
                line: number,
                question: None,
                options: None,
                issue_base,
            },
        });
    }

    fn close(&mut self, block: OpenBlock) {
        match block {
            OpenBlock::Call {
                line,
                tool,
                args,
                issue_base,
            } => {
                match find(&tool) {
                    None => self
                        .issues
                        .push(format!("line {line}: unknown tool {}", shown(&tool))),
                    Some(spec) => validate_args(spec, line, &args, &mut self.issues),
                }
                if self.issues.len() == issue_base {
                    self.items.push(ReplyItem::Call {
                        call: parsed_call_t { tool, args },
                    });
                }
            }
            OpenBlock::Ask {
                line,
                question,
                options,
                issue_base,
            } => {
                match &question {
                    None => self
                        .issues
                        .push(format!("line {line}: @ask has no question:")),
                    Some(text) if text.trim().is_empty() => self
                        .issues
                        .push(format!("line {line}: @ask has an empty question")),
                    Some(_) => {}
                }
                match &options {
                    None => self
                        .issues
                        .push(format!("line {line}: @ask has no options:")),
                    Some(list) if list.len() < 2 => self.issues.push(format!(
                        "line {line}: @ask needs at least 2 options, found {}",
                        list.len()
                    )),
                    Some(_) => {}
                }
                if self.issues.len() == issue_base
                    && let (Some(question), Some(options)) = (question, options)
                {
                    self.items.push(ReplyItem::Ask {
                        question: question_t { question, options },
                    });
                }
            }
        }
    }

    /// Handles one line inside a block and returns the block.
    fn add_line(&mut self, mut block: OpenBlock, number: usize, raw: &str) -> OpenBlock {
        if raw.trim().is_empty() {
            return block;
        }
        match raw.split_once(':') {
            None => self.issues.push(format!(
                "line {number}: no `:` in the block opened on line {}",
                block.line()
            )),
            Some((key, value)) => {
                let value = value.strip_prefix(' ').unwrap_or(value);
                self.keyed_line(&mut block, number, key, value);
            }
        }
        block
    }

    fn keyed_line(&mut self, block: &mut OpenBlock, number: usize, key: &str, value: &str) {
        match block {
            OpenBlock::Call { tool, args, .. } => {
                if !is_snake_name(key) {
                    self.issues.push(format!(
                        "line {number}: call {tool}: argument name {} is not snake_case",
                        shown(key)
                    ));
                    return;
                }
                match unescape(value) {
                    Ok(value) => args.push(tool_arg_t {
                        name: key.to_string(),
                        value,
                    }),
                    Err(problem) => self.issues.push(format!(
                        "line {number}: call {tool}: argument {key}: {problem}"
                    )),
                }
            }
            OpenBlock::Ask {
                question, options, ..
            } => {
                let text = match unescape(value) {
                    Ok(text) => text,
                    Err(problem) => {
                        self.issues
                            .push(format!("line {number}: @ask: {key}: {problem}"));
                        return;
                    }
                };
                match key {
                    "question" if question.is_none() => *question = Some(text),
                    "options" if options.is_none() => {
                        *options = Some(split_options(&text, number, &mut self.issues));
                    }
                    "question" | "options" => self
                        .issues
                        .push(format!("line {number}: @ask: duplicate {key}:")),
                    _ => self
                        .issues
                        .push(format!("line {number}: @ask: unknown key {}", shown(key))),
                }
            }
        }
    }

    /// Turns the pending text lines into one text item, without the blank lines at either end.
    fn flush_text(&mut self) {
        let lines = std::mem::take(&mut self.pending);
        let first = lines.iter().position(|line| !line.trim().is_empty());
        let last = lines.iter().rposition(|line| !line.trim().is_empty());
        if let (Some(first), Some(last)) = (first, last)
            && let Some(span) = lines.get(first..=last)
        {
            self.items.push(ReplyItem::Text {
                text: span.join("\n"),
            });
        }
    }

    fn finish(mut self) -> parsed_reply_t {
        if let Some(block) = self.block.take() {
            self.issues.push(format!(
                "line {}: the block is not closed by @end",
                block.line()
            ));
        }
        self.flush_text();
        let has_call = self
            .items
            .iter()
            .any(|item| matches!(item, ReplyItem::Call { .. }));
        let mut ignored_trailing_text = false;
        if has_call
            && let Some(last_block) = self
                .items
                .iter()
                .rposition(|item| !matches!(item, ReplyItem::Text { .. }))
        {
            let before = self.items.len();
            self.items.truncate(last_block + 1);
            ignored_trailing_text = self.items.len() != before;
        }
        if self.items.is_empty() && self.issues.is_empty() {
            self.issues.push("empty reply".to_string());
        }
        parsed_reply_t {
            items: self.items,
            issues: self.issues,
            ignored_trailing_text,
        }
    }
}

/// Checks a call's arguments against its tool: names, duplicates, lengths and required ones.
fn validate_args(spec: &tool_spec_t, line: usize, args: &[tool_arg_t], issues: &mut Vec<String>) {
    let tool = spec.name;
    let mut seen: Vec<&str> = Vec::new();
    for arg in args {
        let name = arg.name.as_str();
        match spec.args.iter().find(|known| known.name == name) {
            None => issues.push(format!("line {line}: call {tool}: unknown argument {name}")),
            Some(known) if arg.value.len() > known.max_len => issues.push(format!(
                "line {line}: call {tool}: argument {name} is {} bytes, longer than {}",
                arg.value.len(),
                known.max_len
            )),
            Some(_) => {}
        }
        if seen.contains(&name) {
            issues.push(format!(
                "line {line}: call {tool}: duplicate argument {name}"
            ));
        } else {
            seen.push(name);
        }
    }
    for known in spec.args.iter().filter(|known| known.required) {
        if !args.iter().any(|arg| arg.name == known.name) {
            issues.push(format!(
                "line {line}: call {tool}: missing required argument {}",
                known.name
            ));
        }
    }
}

/// Splits the `options:` value at `|`. Each option is trimmed, and an empty one is an issue.
fn split_options(text: &str, number: usize, issues: &mut Vec<String>) -> Vec<String> {
    let mut options = Vec::new();
    for part in text.split('|') {
        let option = part.trim();
        if option.is_empty() {
            issues.push(format!("line {number}: @ask: an option is empty"));
        } else {
            options.push(option.to_string());
        }
    }
    options
}

/// Reads `\n` as a newline and `\\` as a backslash. Any other escape, or a lone backslash at
/// the end, is an error.
fn unescape(value: &str) -> Result<String, String> {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                return Err(format!("unknown escape {}", shown(&format!("\\{other}"))));
            }
            None => return Err("a backslash ends the value".to_string()),
        }
    }
    Ok(out)
}

/// True for a name that starts with a lowercase letter and holds only lowercase letters, digits
/// and underscores.
fn is_snake_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '_'))
}

/// Quotes an untrusted fragment and cuts it to `ShownLimit` characters.
fn shown(text: &str) -> String {
    let body: String = text.chars().take(ShownLimit).collect();
    let more = if text.chars().count() > ShownLimit {
        "..."
    } else {
        ""
    };
    format!("{body:?}{more}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call_items(reply: &parsed_reply_t) -> Vec<&parsed_call_t> {
        reply
            .items
            .iter()
            .filter_map(|item| match item {
                ReplyItem::Call { call } => Some(call),
                _ => None,
            })
            .collect()
    }

    fn arg<'a>(call: &'a parsed_call_t, name: &str) -> Option<&'a str> {
        call.args
            .iter()
            .find(|arg| arg.name == name)
            .map(|arg| arg.value.as_str())
    }

    fn has_issue(reply: &parsed_reply_t, fragment: &str) -> bool {
        reply.issues.iter().any(|issue| issue.contains(fragment))
    }

    #[test]
    fn plain_text_is_one_text_item_and_valid() {
        let reply = parse_reply("Hello.\nSecond line.\n");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert_eq!(
            reply.items,
            vec![ReplyItem::Text {
                text: "Hello.\nSecond line.".to_string()
            }]
        );
        assert!(!reply.ignored_trailing_text);
    }

    #[test]
    fn a_valid_call_block_parses_with_its_args_in_order() {
        let reply = parse_reply("@call read_file\npath: src/main.rs\nlines: 1 80\n@end");
        assert!(reply.valid(), "{:?}", reply.issues);
        let calls = call_items(&reply);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool, "read_file");
        assert_eq!(arg(calls[0], "path"), Some("src/main.rs"));
        assert_eq!(arg(calls[0], "lines"), Some("1 80"));
        assert_eq!(calls[0].args[0].name, "path");
        assert_eq!(calls[0].args[1].name, "lines");
    }

    #[test]
    fn text_after_the_last_block_is_dropped_when_a_call_is_valid() {
        let reply =
            parse_reply("Let me look.\n@call read_file\npath: a\n@end\nI have the result.\n");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert!(reply.ignored_trailing_text);
        assert_eq!(reply.items.len(), 2);
        assert_eq!(
            reply.items[0],
            ReplyItem::Text {
                text: "Let me look.".to_string()
            }
        );
        assert!(matches!(reply.items[1], ReplyItem::Call { .. }));
    }

    #[test]
    fn text_between_blocks_keeps_its_order() {
        let reply =
            parse_reply("@call list_dir\npath: .\n@end\nbetween\n@call read_file\npath: a\n@end\n");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert_eq!(reply.items.len(), 3);
        assert!(matches!(reply.items[0], ReplyItem::Call { .. }));
        assert_eq!(
            reply.items[1],
            ReplyItem::Text {
                text: "between".to_string()
            }
        );
        assert!(matches!(reply.items[2], ReplyItem::Call { .. }));
        assert!(!reply.ignored_trailing_text);
    }

    #[test]
    fn text_after_an_ask_block_is_kept_when_no_call_is_present() {
        let reply = parse_reply("@ask\nquestion: Which?\noptions: a | b\n@end\nafter\n");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert!(!reply.ignored_trailing_text);
        assert_eq!(reply.items.len(), 2);
        match &reply.items[0] {
            ReplyItem::Ask { question } => {
                assert_eq!(question.question, "Which?");
                assert_eq!(question.options, vec!["a".to_string(), "b".to_string()]);
            }
            other => panic!("expected an ask item, found {other:?}"),
        }
    }

    #[test]
    fn escapes_are_read_in_values() {
        let reply =
            parse_reply("@call replace_lines\npath: a\nstart: 1\nend: 1\ntext: x\\ny\\\\z\n@end");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert_eq!(arg(call_items(&reply)[0], "text"), Some("x\ny\\z"));
    }

    #[test]
    fn an_unknown_escape_is_an_issue_and_the_call_is_dropped() {
        let reply =
            parse_reply("@call replace_lines\npath: a\nstart: 1\nend: 1\ntext: a\\tb\n@end");
        assert!(!reply.valid());
        assert!(call_items(&reply).is_empty());
        assert!(has_issue(&reply, "unknown escape"));
    }

    #[test]
    fn a_trailing_backslash_is_an_issue() {
        let reply = parse_reply("@call search\npattern: a\\\npath: .\n@end");
        assert!(has_issue(&reply, "a backslash ends the value"));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn one_space_is_stripped_after_the_colon() {
        let reply = parse_reply("@call search\npattern:  two\npath:src\n@end");
        assert!(reply.valid(), "{:?}", reply.issues);
        let calls = call_items(&reply);
        assert_eq!(arg(calls[0], "pattern"), Some(" two"));
        assert_eq!(arg(calls[0], "path"), Some("src"));
    }

    #[test]
    fn an_unknown_tool_name_is_an_issue_with_no_fuzzy_match() {
        let reply = parse_reply("@call Read_File\npath: a\n@end");
        assert!(!reply.valid());
        assert!(has_issue(&reply, "unknown tool \"Read_File\""));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn a_missing_required_argument_is_named() {
        let reply = parse_reply("@call read_file\nlines: 1 2\n@end");
        assert!(has_issue(&reply, "missing required argument path"));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn an_unknown_argument_is_named() {
        let reply = parse_reply("@call read_file\npath: a\nmode: fast\n@end");
        assert!(has_issue(&reply, "unknown argument mode"));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn a_duplicate_argument_is_named() {
        let reply = parse_reply("@call read_file\npath: a\npath: b\n@end");
        assert!(has_issue(&reply, "duplicate argument path"));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn a_value_longer_than_its_limit_is_an_issue() {
        let at_limit = "p".repeat(256);
        let ok = parse_reply(&format!("@call read_file\npath: {at_limit}\n@end"));
        assert!(ok.valid(), "{:?}", ok.issues);
        let too_long = "p".repeat(257);
        let reply = parse_reply(&format!("@call read_file\npath: {too_long}\n@end"));
        assert!(has_issue(
            &reply,
            "argument path is 257 bytes, longer than 256"
        ));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn an_unterminated_block_is_an_issue() {
        let reply = parse_reply("Start.\n@call read_file\npath: a\n");
        assert!(!reply.valid());
        assert!(has_issue(&reply, "the block is not closed by @end"));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn a_nested_call_drops_the_outer_block_and_keeps_the_inner_one() {
        let reply = parse_reply("@call read_file\n@call list_dir\npath: .\n@end");
        assert!(!reply.valid());
        assert!(has_issue(
            &reply,
            "opens before the @end of the block from line 1"
        ));
        let calls = call_items(&reply);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool, "list_dir");
    }

    #[test]
    fn an_ask_inside_a_call_is_an_issue() {
        let reply = parse_reply("@call list_dir\n@ask\nquestion: q\noptions: a | b\n@end");
        assert!(has_issue(&reply, "opens before the @end"));
        assert!(matches!(reply.items.as_slice(), [ReplyItem::Ask { .. }]));
    }

    #[test]
    fn a_stray_end_is_an_issue() {
        let reply = parse_reply("text\n@end\n");
        assert!(has_issue(&reply, "@end without an open block"));
        assert!(!reply.valid());
    }

    #[test]
    fn a_line_without_a_colon_in_a_block_is_an_issue() {
        let reply = parse_reply("@call list_dir\npath .\n@end");
        assert!(has_issue(&reply, "no `:` in the block opened on line 1"));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn a_key_that_is_not_snake_case_is_an_issue() {
        let reply = parse_reply("@call list_dir\nPath: a\n@end");
        assert!(has_issue(
            &reply,
            "argument name \"Path\" is not snake_case"
        ));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn blank_lines_inside_a_block_are_ignored() {
        let reply = parse_reply("@call list_dir\n\npath: .\n\n@end");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert_eq!(call_items(&reply).len(), 1);
    }

    #[test]
    fn markers_may_have_trailing_whitespace_but_not_leading() {
        let reply = parse_reply("@call list_dir   \npath: .\n@end\t \n");
        assert!(reply.valid(), "{:?}", reply.issues);
        let indented = parse_reply("  @call list_dir\npath: .\n@end");
        assert!(
            indented
                .items
                .iter()
                .all(|item| matches!(item, ReplyItem::Text { .. }))
        );
        assert!(has_issue(&indented, "@end without an open block"));
    }

    #[test]
    fn a_bare_call_marker_names_no_tool() {
        let reply = parse_reply("@call\n@end");
        assert!(has_issue(&reply, "unknown tool \"\""));
        assert!(call_items(&reply).is_empty());
    }

    #[test]
    fn a_valid_ask_block_parses() {
        let reply = parse_reply("@ask\nquestion: Pick one\noptions: yes | no | later\n@end");
        assert!(reply.valid(), "{:?}", reply.issues);
        match reply.items.as_slice() {
            [ReplyItem::Ask { question }] => {
                assert_eq!(question.question, "Pick one");
                assert_eq!(question.options.len(), 3);
            }
            other => panic!("expected one ask item, found {other:?}"),
        }
    }

    #[test]
    fn an_ask_needs_two_options_and_no_empty_option() {
        let one = parse_reply("@ask\nquestion: q\noptions: only\n@end");
        assert!(has_issue(&one, "needs at least 2 options, found 1"));
        assert!(one.items.is_empty());
        let empty = parse_reply("@ask\nquestion: q\noptions: a || b\n@end");
        assert!(has_issue(&empty, "an option is empty"));
        assert!(empty.items.is_empty());
        let trailing = parse_reply("@ask\nquestion: q\noptions: a | b |\n@end");
        assert!(has_issue(&trailing, "an option is empty"));
    }

    #[test]
    fn an_ask_with_a_missing_or_unknown_part_is_an_issue() {
        let no_question = parse_reply("@ask\noptions: a | b\n@end");
        assert!(has_issue(&no_question, "has no question"));
        let no_options = parse_reply("@ask\nquestion: q\n@end");
        assert!(has_issue(&no_options, "has no options"));
        let unknown = parse_reply("@ask\nquestion: q\noptions: a | b\nhint: c\n@end");
        assert!(has_issue(&unknown, "unknown key \"hint\""));
        let duplicate = parse_reply("@ask\nquestion: q\nquestion: r\noptions: a | b\n@end");
        assert!(has_issue(&duplicate, "duplicate question:"));
        assert!(duplicate.items.is_empty());
    }

    #[test]
    fn an_ask_block_is_closed_by_its_end_and_text_resumes() {
        let reply =
            parse_reply("@ask\nquestion: q\noptions: a | b\n@end\n@call list_dir\npath: .\n@end");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert_eq!(reply.items.len(), 2);
    }

    #[test]
    fn an_empty_reply_is_invalid_with_a_named_issue() {
        for text in ["", "   \n\n\t\n"] {
            let reply = parse_reply(text);
            assert!(!reply.valid());
            assert!(reply.items.is_empty());
            assert_eq!(reply.issues, vec!["empty reply".to_string()]);
        }
    }

    #[test]
    fn crlf_line_endings_parse_like_lf() {
        let reply = parse_reply("@call read_file\r\npath: a\r\n@end\r\n");
        assert!(reply.valid(), "{:?}", reply.issues);
        assert_eq!(arg(call_items(&reply)[0], "path"), Some("a"));
    }

    #[test]
    fn unicode_nul_and_a_huge_line_do_not_panic() {
        let text = "héllo \u{1F600} \0 end\n";
        let reply = parse_reply(text);
        assert_eq!(
            reply.items,
            vec![ReplyItem::Text {
                text: "héllo \u{1F600} \0 end".to_string()
            }]
        );
        let huge = "x".repeat(200_000);
        let big = parse_reply(&format!("@call search\npattern: {huge}\npath: .\n@end"));
        assert!(has_issue(&big, "argument pattern is 200000 bytes"));
        let big_text = parse_reply(&huge);
        assert!(big_text.valid());
    }

    #[test]
    fn malformed_input_yields_issues_not_panics() {
        let inputs = [
            "@call",
            "@call \n@end",
            "@end",
            "@ask",
            "@call read_file\n:\n@end",
            "@call read_file\n\u{0}: x\n@end",
            "@call read_file\npath: \\\n@end",
            "@call\n@call\n@ask\n@end\n@end",
            "@ask\n@end",
            "@call read_file\npath: a\n@call read_file\npath: b\n@end",
            "\u{1F600}\n\u{0}\n@end\n@call ",
        ];
        for input in inputs {
            let reply = parse_reply(input);
            if reply.items.is_empty() {
                assert!(!reply.valid(), "{input:?} must be invalid");
            }
        }
    }

    #[test]
    fn an_issue_quotes_and_cuts_untrusted_text() {
        let long_name = format!("{}\u{1b}", "n".repeat(60));
        let reply = parse_reply(&format!("@call {long_name}\n@end"));
        let issue = reply.issues.first().cloned().unwrap_or_default();
        assert!(issue.contains("..."));
        assert!(!issue.contains('\u{1b}'));
    }
}
