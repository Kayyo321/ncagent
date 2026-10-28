/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Scripted replies for the demo backend (PROJECT.md, "UI prototype (P1)", "Demo backend").
//!
//! A reply is a pure function of the prefix text and the seed. The prefix uses the template of
//! D43. The demo finds the last `User: ` line, picks a scenario from a trigger word in that
//! message, and counts the `Assistant: ` lines after it to choose the step. Nothing here runs a
//! command, reads a file or contacts a service. Every reply is simulated output.

/// Marks a user message in the packed prefix (D43).
const UserMark: &str = "User: ";
/// Marks a reply of the assistant in the packed prefix (D43).
const AssistantMark: &str = "Assistant: ";
/// Marks the answer to a question in the packed prefix (D43).
const AnswerMark: &str = "User answer: ";
/// Marks a tool result, and a tool error, in the packed prefix (D43).
const ToolMark: &str = "Tool ";
/// The open line at the end of the packed prefix (D43).
const OpenAssistant: &str = "Assistant:";

/// The scenario used when no trigger word is found.
const DefaultScenario: &str = "default";

/// Trigger words in priority order, each with the scenario it selects. `deny` and `approve`
/// select the same scenario.
const Triggers: [(&str, &str); 10] = [
    ("error", "error"),
    ("flaky", "flaky"),
    ("long", "long"),
    ("markdown", "markdown"),
    ("fail", "fail"),
    ("shell", "shell"),
    ("approve", "approve"),
    ("deny", "approve"),
    ("ask", "ask"),
    ("tool", "tool"),
];

/// Longest answer text the demo repeats, in characters.
const AnswerLimit: usize = 80;

/// The message of the scripted backend failure of the `error` scenario.
const ErrorMessage: &str = "simulated backend failure: scripted scenario `error`";

/// What a scripted call produces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScriptedOutcome {
    /// The reply text. The backend encodes it with the byte codec.
    Reply(String),
    /// The backend fails after the level pass `level` (0-based), with `message`.
    FailAfterLevel { level: u32, message: String },
}

/// The scripted answer to one call, with the scenario and step that chose it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct scripted_t {
    /// The scenario name, such as `tool` or `default`.
    pub scenario: &'static str,
    /// How many `Assistant: ` lines follow the user message that chose the scenario.
    pub step: usize,
    pub outcome: ScriptedOutcome,
}

/// The scripted reply for a call.
///
/// `prefix_text` is the decoded prefix, and `seed` is the call's seed. Only the `flaky` scenario
/// reads the seed (see its doc comment). Malformed or empty text selects the default scenario.
pub fn scripted_reply(prefix_text: &str, seed: u64) -> scripted_t {
    let ctx = analyze(prefix_text);
    let scenario = select_scenario(ctx.user_text.as_deref());
    let outcome = outcome_for(scenario, ctx.step, &ctx, seed);
    scripted_t {
        scenario,
        step: ctx.step,
        outcome,
    }
}

/// What the prefix says about the current turn.
#[derive(Debug, Default, PartialEq, Eq)]
struct context_t {
    /// The text of the last user message. `None` when the prefix has no user line.
    user_text: Option<String>,
    /// `Assistant: ` lines after that user message.
    step: usize,
    /// The last `Tool ` line after that user message, or empty.
    last_result: String,
    /// The text after the last `User answer: ` line after that user message.
    last_answer: Option<String>,
}

/// True for a line that starts a new template item, so it cannot continue a user message.
fn is_marker(line: &str) -> bool {
    line.starts_with(UserMark)
        || line.starts_with(AssistantMark)
        || line.starts_with(AnswerMark)
        || line.starts_with(ToolMark)
        || line == OpenAssistant
}

fn analyze(prefix_text: &str) -> context_t {
    let lines: Vec<&str> = prefix_text.lines().collect();
    let Some(user_at) = lines.iter().rposition(|line| line.starts_with(UserMark)) else {
        return context_t::default();
    };

    // The user text is the rest of the `User: ` line and any lines up to the next template item.
    let mut user_text = String::new();
    user_text.push_str(
        lines
            .get(user_at)
            .and_then(|line| line.strip_prefix(UserMark))
            .unwrap_or(""),
    );
    let mut after = user_at + 1;
    while let Some(line) = lines.get(after) {
        if is_marker(line) {
            break;
        }
        user_text.push('\n');
        user_text.push_str(line);
        after += 1;
    }

    let tail = lines.get(after..).unwrap_or(&[]);
    let step = tail
        .iter()
        .filter(|line| line.starts_with(AssistantMark))
        .count();
    let last_result = tail
        .iter()
        .rev()
        .find(|line| line.starts_with(ToolMark))
        .copied()
        .unwrap_or("")
        .to_string();
    let last_answer = tail
        .iter()
        .rev()
        .find_map(|line| line.strip_prefix(AnswerMark))
        .map(str::to_string);

    context_t {
        user_text: Some(user_text),
        step,
        last_result,
        last_answer,
    }
}

/// The scenario of the first trigger word, in priority order, that the user text contains as a
/// whole word. Matching ignores case and splits on every non-alphanumeric character.
fn select_scenario(user_text: Option<&str>) -> &'static str {
    let Some(text) = user_text else {
        return DefaultScenario;
    };
    let lowered = text.to_lowercase();
    let words: Vec<&str> = lowered
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    Triggers
        .iter()
        .find(|(word, _)| words.contains(word))
        .map(|(_, scenario)| *scenario)
        .unwrap_or(DefaultScenario)
}

fn outcome_for(scenario: &str, step: usize, ctx: &context_t, seed: u64) -> ScriptedOutcome {
    match scenario {
        "tool" => tool_reply(step),
        "approve" => approve_reply(step, &ctx.last_result),
        "shell" => shell_reply(step),
        "ask" => ask_reply(step, ctx.last_answer.as_deref()),
        "fail" => fail_reply(step),
        "error" => ScriptedOutcome::FailAfterLevel {
            level: 1,
            message: ErrorMessage.to_string(),
        },
        "flaky" => flaky_reply(step, seed),
        "long" => single_reply(step, LongReply),
        "markdown" => single_reply(step, MarkdownReply),
        _ => single_reply(step, DefaultReply),
    }
}

fn reply(text: &str) -> ScriptedOutcome {
    ScriptedOutcome::Reply(text.to_string())
}

/// The closing reply for a step past the end of a script.
fn closing() -> ScriptedOutcome {
    reply(ClosingReply)
}

/// A scenario that answers once: its text at step 0, then the closing reply.
fn single_reply(step: usize, text: &str) -> ScriptedOutcome {
    if step == 0 { reply(text) } else { closing() }
}

fn tool_reply(step: usize) -> ScriptedOutcome {
    match step {
        0 => reply(ToolStepZero),
        1 => reply(ToolStepOne),
        2 => reply(ToolSummary),
        _ => closing(),
    }
}

/// `approve` and `deny` share this scenario. At step 1 the last tool result decides the reply.
fn approve_reply(step: usize, last_result: &str) -> ScriptedOutcome {
    match step {
        0 => reply(ApproveStepZero),
        1 if is_denial(last_result) => reply(ApproveDenied),
        1 => reply(ApproveApplied),
        _ => closing(),
    }
}

/// A tool error whose message says the user denied the call.
fn is_denial(last_result: &str) -> bool {
    last_result.starts_with("Tool error") && last_result.to_lowercase().contains("denied")
}

fn shell_reply(step: usize) -> ScriptedOutcome {
    match step {
        0 => reply(ShellStepZero),
        1 => reply(ShellSummary),
        _ => closing(),
    }
}

/// The question is fixed. At step 1 the reply repeats the answer from the `User answer: ` line.
fn ask_reply(step: usize, last_answer: Option<&str>) -> ScriptedOutcome {
    match (step, last_answer) {
        (0, _) => reply(AskStepZero),
        (1, Some(answer)) => reply(&format!(
            "You chose \"{}\". The demo uses that answer for the next step, and nothing else was decided.",
            answer_text(answer)
        )),
        (1, None) => reply(AskNoAnswer),
        _ => closing(),
    }
}

/// The answer as shown in a reply: cut short, and with `@` removed so it cannot start a block.
fn answer_text(answer: &str) -> String {
    answer
        .chars()
        .filter(|c| *c != '@')
        .take(AnswerLimit)
        .collect()
}

fn fail_reply(step: usize) -> ScriptedOutcome {
    match step {
        0 => reply(FailStepZero),
        1 => reply(FailStepOne),
        _ => closing(),
    }
}

/// `flaky` is coupled to the seed policy of D42. A call's first sample uses an even seed, and a
/// retry uses the next odd seed. So with an even seed, step 0 returns a reply with an unknown
/// tool, which the parser rejects, and with an odd seed it returns a valid reply. Later steps
/// are valid.
fn flaky_reply(step: usize, seed: u64) -> ScriptedOutcome {
    match step {
        0 if seed.is_multiple_of(2) => reply(FlakyInvalid),
        0 => reply(FlakyValidFirst),
        1 => reply(FlakyValidLater),
        _ => closing(),
    }
}

const DefaultReply: &str = "## Demo reply

This is the **DEMO** simulator, so no model is running and nothing was executed.

Try one of: `tool`, `approve`, `deny`, `shell`, `ask`, `fail`, `error`, `flaky`, `long`, `markdown`.";

const ClosingReply: &str =
    "The scripted reply for this step is finished. Send a new message to start another scenario.";

const ToolStepZero: &str = r##"I will list the project folder first.

@call list_dir
path: .
@end"##;

const ToolStepOne: &str = r##"The listing came back as simulated text. Next I will read the start of `src/main.rs`.

@call read_file
path: src/main.rs
lines: 1 20
@end"##;

const ToolSummary: &str = "## Summary (simulated)

The simulated listing showed `Cargo.toml`, `README.md` and a `src/` folder. The simulated excerpt of `src/main.rs` was fixed demo text.

Nothing was read from your disk. Both results came from the demo tool runner.";

const ApproveStepZero: &str = r##"I will make one small edit to `src/lib.rs`. The edit needs your approval first.

@call replace_lines
path: src/lib.rs
start: 3
end: 4
text: // demo edit\nlet x = 1;
@end"##;

const ApproveDenied: &str = "The edit was denied, so I did not change `src/lib.rs`. Nothing was written, and the file stays as it was.";

const ApproveApplied: &str = "The simulated edit was applied in the demo. Nothing was written to disk, because the demo never changes project files.";

const ShellStepZero: &str = r##"I will run one harmless command.

@call run_shell
command: echo "hello from the demo"
@end"##;

const ShellSummary: &str = "The simulated shell printed the echoed line. No command ran on this machine, so the output is fixed demo text.";

const AskStepZero: &str = r##"I need one choice before I continue.

@ask
question: Which granularity should the demo use?
options: coarse | fine | both
@end"##;

const AskNoAnswer: &str = "No answer line was found, so the demo keeps its default.";

const FailStepZero: &str = r##"I will try to read a file that may not exist.

@call read_file
path: docs/missing.txt
@end"##;

const FailStepOne: &str = "The read of `docs/missing.txt` failed, so the simulated tool reported no such file. A next step is to list the `docs` folder, or to ask you where the file lives.";

const FlakyInvalid: &str = r##"Let me check the folder.

@call no_such_tool
path: .
@end"##;

const FlakyValidFirst: &str = "This is a valid reply from the flaky scenario. Even seeds send an invalid sample first (D42), and odd seeds send this valid one.";

const FlakyValidLater: &str = "The flaky scenario is valid from here on, so this reply parses.";

const LongReply: &str = "# Long reply from the demo

This reply exists to show how a long answer is laid out. The text is scripted and written by hand, and no model produced it.

## Why it is long

Under the standard profile the reply length L is 1024 positions, so this whole reply fits in one call. Under the compact profile L is 256 positions. The same reply is then cut before its end, and the transcript marks it as truncated. That cut is the only difference between the two profiles for this text.

## What to look for

Watch the level strip fill from the smallest level to the largest. Each level pass in this demo is a delay chosen by the speed flag, not a measurement of anything.

## Last paragraph

Send one of the other trigger words to see a different scenario. Nothing in this text is a benchmark, and no confidence figure is attached to it.";

const MarkdownReply: &str = "# Markdown showcase

## Inline styles

This line has **bold**, *italic* and `inline code`. The link goes to the [NCM paper](https://arxiv.org/abs/2610.14162). Everything here is scripted demo text.

## Lists

- A bullet point
- A second bullet with `code`

1. First numbered item
2. Second numbered item

> A block quote. It is scripted text for the demo, and it shows no result.

```rust
fn simulated() -> &'static str {
    \"no model ran\"
}
```

The table below lists the parts of this demo. Each part is marked simulated, because none of them runs a model or a tool.

| Part | Source | Status |
|---|---|---|
| Tokens | byte codec | simulated |
| Levels | scripted | simulated |
| Tools | fixed text | simulated |";

#[cfg(test)]
mod tests {
    use super::*;
    use harness::tools::find;

    fn run(lines: &[&str]) -> scripted_t {
        scripted_reply(&packed(lines), 4242)
    }

    /// The packed prefix of D43 for these lines: each line ends with a newline, and the open
    /// `Assistant:` line closes it.
    fn packed(lines: &[&str]) -> String {
        let mut text = String::new();
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str(OpenAssistant);
        text
    }

    /// One user message followed by `steps` assistant turns, each with a tool result.
    fn history(user: &str, steps: usize) -> Vec<String> {
        let mut lines = vec![format!("User: {user}")];
        for index in 0..steps {
            lines.push(format!("Assistant: step {index}"));
            lines.push(format!("Tool list_dir: result {index}"));
        }
        lines
    }

    fn run_owned(lines: &[String], seed: u64) -> scripted_t {
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        scripted_reply(&packed(&borrowed), seed)
    }

    fn text_of(scripted: &scripted_t) -> &str {
        match &scripted.outcome {
            ScriptedOutcome::Reply(text) => text,
            ScriptedOutcome::FailAfterLevel { .. } => "",
        }
    }

    #[test]
    fn empty_prefix_uses_the_default_scenario() {
        let scripted = scripted_reply("", 4242);
        assert_eq!(scripted.scenario, "default");
        assert_eq!(scripted.step, 0);
        assert_eq!(scripted.outcome, reply(DefaultReply));
    }

    #[test]
    fn default_reply_names_the_simulator_and_fits_the_compact_window() {
        assert!(DefaultReply.len() <= 230, "{} bytes", DefaultReply.len());
        assert!(DefaultReply.contains("DEMO"));
        assert!(DefaultReply.contains("no model is running"));
        for word in [
            "tool", "approve", "deny", "shell", "ask", "fail", "error", "flaky", "long", "markdown",
        ] {
            assert!(DefaultReply.contains(word), "missing trigger {word}");
        }
    }

    #[test]
    fn each_trigger_word_selects_its_scenario() {
        let cases = [
            ("tool", "tool"),
            ("approve", "approve"),
            ("deny", "approve"),
            ("shell", "shell"),
            ("ask", "ask"),
            ("fail", "fail"),
            ("error", "error"),
            ("flaky", "flaky"),
            ("long", "long"),
            ("markdown", "markdown"),
        ];
        for (word, scenario) in cases {
            let line = format!("User: please {word} now");
            assert_eq!(run(&[line.as_str()]).scenario, scenario, "trigger {word}");
        }
    }

    #[test]
    fn trigger_words_match_whole_words_ignoring_case_and_punctuation() {
        assert_eq!(run(&["User: TOOL"]).scenario, "tool");
        assert_eq!(run(&["User: Tool, please."]).scenario, "tool");
        assert_eq!(run(&["User: tools"]).scenario, "default");
        assert_eq!(run(&["User: a failure"]).scenario, "default");
        assert_eq!(run(&["User: longer text"]).scenario, "default");
    }

    #[test]
    fn priority_order_decides_between_trigger_words() {
        assert_eq!(run(&["User: tool and error"]).scenario, "error");
        assert_eq!(run(&["User: deny then flaky"]).scenario, "flaky");
        assert_eq!(run(&["User: ask about long"]).scenario, "long");
        assert_eq!(run(&["User: fail with shell"]).scenario, "fail");
        assert_eq!(run(&["User: approve or deny"]).scenario, "approve");
    }

    #[test]
    fn only_the_last_user_line_decides() {
        let scripted = run(&["User: tool", "Assistant: hi", "User: hello"]);
        assert_eq!(scripted.scenario, "default");
        assert_eq!(scripted.step, 0);
    }

    #[test]
    fn a_user_answer_line_is_not_a_user_message() {
        assert_eq!(run(&["User answer: tool"]).scenario, "default");
    }

    #[test]
    fn text_without_a_user_line_uses_the_default() {
        let scripted = scripted_reply("Assistant: hi\nTool list_dir: x\nAssistant:", 1);
        assert_eq!(scripted.scenario, "default");
        assert_eq!(scripted.step, 0);
    }

    #[test]
    fn malformed_user_lines_do_not_panic_and_fall_back() {
        assert_eq!(scripted_reply("User:", 1).scenario, "default");
        assert_eq!(scripted_reply("User: ", 1).scenario, "default");
        assert_eq!(scripted_reply("user: tool", 1).scenario, "default");
        assert_eq!(
            scripted_reply("User: \u{0}\u{1F600}", 1).scenario,
            "default"
        );
        assert_eq!(scripted_reply("\n\n\n", 1).scenario, "default");
        assert_eq!(scripted_reply("Assistant:", 1).scenario, "default");
    }

    #[test]
    fn non_ascii_text_still_finds_a_trigger_word() {
        assert_eq!(run(&["User: héllo ✓ tool"]).scenario, "tool");
    }

    #[test]
    fn continuation_lines_belong_to_the_user_message() {
        let scripted = run(&["User: first line", "tool continues here"]);
        assert_eq!(scripted.scenario, "tool");
    }

    #[test]
    fn step_counts_assistant_lines_after_the_user_line() {
        let scripted = run(&[
            "Assistant: before the user line",
            "User: tool",
            "Assistant: a",
            "Tool list_dir: x",
            "Assistant: b",
            "Tool read_file: y",
        ]);
        assert_eq!(scripted.scenario, "tool");
        assert_eq!(scripted.step, 2);
    }

    #[test]
    fn tool_scenario_steps() {
        let step0 = run_owned(&history("tool", 0), 1);
        assert!(text_of(&step0).contains("@call list_dir\npath: .\n@end"));

        let step1 = run_owned(&history("tool", 1), 1);
        assert!(text_of(&step1).contains("@call read_file\npath: src/main.rs\nlines: 1 20\n@end"));

        let step2 = run_owned(&history("tool", 2), 1);
        assert_eq!(step2.outcome, reply(ToolSummary));

        let step3 = run_owned(&history("tool", 3), 1);
        assert_eq!(step3.outcome, closing());
    }

    #[test]
    fn approve_scenario_asks_for_the_edit_then_reports_the_result() {
        let step0 = run_owned(&history("approve", 0), 1);
        assert!(text_of(&step0).contains(
            "@call replace_lines\npath: src/lib.rs\nstart: 3\nend: 4\ntext: // demo edit\\nlet x = 1;\n@end"
        ));

        let denied = run(&[
            "User: approve",
            "Assistant: edit",
            "Tool error replace_lines: denied by the user",
        ]);
        assert_eq!(denied.step, 1);
        assert_eq!(denied.outcome, reply(ApproveDenied));

        let applied = run(&[
            "User: approve",
            "Assistant: edit",
            "Tool replace_lines: SIMULATED: no file was changed.",
        ]);
        assert_eq!(applied.outcome, reply(ApproveApplied));

        let other_error = run(&[
            "User: approve",
            "Assistant: edit",
            "Tool error replace_lines: no such line",
        ]);
        assert_eq!(other_error.outcome, reply(ApproveApplied));

        let step2 = run(&[
            "User: deny",
            "Assistant: edit",
            "Tool error replace_lines: denied",
            "Assistant: noted",
        ]);
        assert_eq!(step2.outcome, closing());
    }

    #[test]
    fn shell_scenario_runs_one_echo_then_summarises() {
        let step0 = run_owned(&history("shell", 0), 1);
        assert!(
            text_of(&step0)
                .contains("@call run_shell\ncommand: echo \"hello from the demo\"\n@end")
        );
        assert_eq!(
            run_owned(&history("shell", 1), 1).outcome,
            reply(ShellSummary)
        );
        assert_eq!(run_owned(&history("shell", 2), 1).outcome, closing());
    }

    #[test]
    fn ask_scenario_asks_then_repeats_the_answer() {
        let step0 = run_owned(&history("ask", 0), 1);
        let text = text_of(&step0);
        assert!(text.contains("@ask\nquestion: "));
        assert!(text.contains("options: coarse | fine | both\n@end"));

        let answered = run(&["User: ask", "Assistant: question", "User answer: fine"]);
        assert_eq!(answered.step, 1);
        let answered_text = text_of(&answered);
        assert!(answered_text.contains("You chose \"fine\"."));

        let unanswered = run(&["User: ask", "Assistant: question"]);
        assert_eq!(unanswered.outcome, reply(AskNoAnswer));
    }

    #[test]
    fn answer_text_is_cut_and_stripped_of_at_signs() {
        let long: String = "@".repeat(5) + &"x".repeat(200);
        let shown = answer_text(&long);
        assert_eq!(shown.chars().count(), AnswerLimit);
        assert!(!shown.contains('@'));
    }

    #[test]
    fn fail_scenario_reads_a_missing_file_then_recovers() {
        let step0 = run_owned(&history("fail", 0), 1);
        assert!(text_of(&step0).contains("@call read_file\npath: docs/missing.txt\n@end"));
        assert_eq!(
            run_owned(&history("fail", 1), 1).outcome,
            reply(FailStepOne)
        );
        assert_eq!(run_owned(&history("fail", 2), 1).outcome, closing());
    }

    #[test]
    fn error_scenario_fails_after_level_one_at_every_step() {
        for steps in 0..4 {
            let scripted = run_owned(&history("error", steps), 1);
            assert_eq!(scripted.scenario, "error");
            assert_eq!(
                scripted.outcome,
                ScriptedOutcome::FailAfterLevel {
                    level: 1,
                    message: ErrorMessage.to_string(),
                }
            );
        }
        assert!(ErrorMessage.contains("simulated backend failure"));
    }

    #[test]
    fn flaky_scenario_is_invalid_on_even_seeds_and_valid_on_odd_seeds() {
        let even = run_owned(&history("flaky", 0), 4242);
        assert!(text_of(&even).contains("@call no_such_tool"));

        let odd = run_owned(&history("flaky", 0), 4243);
        assert_eq!(odd.outcome, reply(FlakyValidFirst));
        assert!(!text_of(&odd).contains("no_such_tool"));

        let later_even = run_owned(&history("flaky", 1), 4242);
        assert_eq!(later_even.outcome, reply(FlakyValidLater));
        let later_odd = run_owned(&history("flaky", 1), 4243);
        assert_eq!(later_odd.outcome, reply(FlakyValidLater));

        assert_eq!(run_owned(&history("flaky", 2), 4242).outcome, closing());
    }

    #[test]
    fn long_scenario_is_long_and_only_fits_the_standard_window() {
        let scripted = run_owned(&history("long", 0), 1);
        assert_eq!(scripted.outcome, reply(LongReply));
        assert!(
            (820..=980).contains(&LongReply.len()),
            "{} bytes",
            LongReply.len()
        );
        assert!(
            LongReply.len() > 256,
            "must be cut under the compact profile"
        );
        assert!(LongReply.len() <= 1023);
        assert_eq!(run_owned(&history("long", 1), 1).outcome, closing());
    }

    #[test]
    fn markdown_scenario_shows_each_element() {
        let scripted = run_owned(&history("markdown", 0), 1);
        assert_eq!(scripted.outcome, reply(MarkdownReply));
        assert!(
            (600..=800).contains(&MarkdownReply.len()),
            "{} bytes",
            MarkdownReply.len()
        );
        for element in [
            "# Markdown showcase",
            "## Inline styles",
            "**bold**",
            "*italic*",
            "`inline code`",
            "- A bullet point",
            "1. First numbered item",
            "> A block quote",
            "```rust",
            "| Part | Source | Status |",
            "](https://arxiv.org/abs/2610.14162)",
        ] {
            assert!(MarkdownReply.contains(element), "missing {element}");
        }
        assert_eq!(run_owned(&history("markdown", 1), 1).outcome, closing());
    }

    #[test]
    fn every_reply_fits_under_the_standard_limit() {
        let triggers = [
            "tool", "approve", "deny", "shell", "ask", "fail", "error", "flaky", "long",
            "markdown", "none",
        ];
        for trigger in triggers {
            for steps in 0..6 {
                for seed in [4242u64, 4243] {
                    let scripted = run_owned(&history(trigger, steps), seed);
                    if let ScriptedOutcome::Reply(text) = &scripted.outcome {
                        assert!(
                            text.len() <= 1023,
                            "{trigger} step {steps} is {} bytes",
                            text.len()
                        );
                    }
                }
            }
        }
        assert!(ClosingReply.len() <= 1023);
    }

    #[test]
    fn replies_never_start_a_template_line() {
        let texts = [
            DefaultReply,
            ToolStepZero,
            ToolStepOne,
            ToolSummary,
            ApproveStepZero,
            ApproveDenied,
            ApproveApplied,
            ShellStepZero,
            ShellSummary,
            AskStepZero,
            AskNoAnswer,
            FailStepZero,
            FailStepOne,
            FlakyInvalid,
            FlakyValidFirst,
            FlakyValidLater,
            LongReply,
            MarkdownReply,
            ClosingReply,
        ];
        for text in texts {
            for line in text.lines() {
                assert!(!is_marker(line), "template-like line: {line:?}");
            }
        }
    }

    #[test]
    fn every_tool_block_matches_the_registry() {
        let texts = [
            ToolStepZero,
            ToolStepOne,
            ApproveStepZero,
            ShellStepZero,
            FailStepZero,
            FlakyInvalid,
        ];
        for text in texts {
            let mut lines = text.lines();
            while let Some(line) = lines.next() {
                let Some(name) = line.strip_prefix("@call ") else {
                    continue;
                };
                let mut keys = Vec::new();
                for arg in lines.by_ref() {
                    if arg == "@end" {
                        break;
                    }
                    let (key, _) = arg.split_once(": ").unwrap_or((arg, ""));
                    keys.push(key.to_string());
                }
                let Some(spec) = find(name) else {
                    assert_eq!(name, "no_such_tool", "unknown tool in a script");
                    continue;
                };
                for key in &keys {
                    assert!(
                        spec.args.iter().any(|arg| arg.name == key.as_str()),
                        "{name} has no argument {key}"
                    );
                }
                for arg in spec.args.iter().filter(|arg| arg.required) {
                    assert!(
                        keys.iter().any(|key| key.as_str() == arg.name),
                        "{name} lacks {}",
                        arg.name
                    );
                }
            }
        }
    }

    #[test]
    fn steps_past_the_script_return_the_closing_reply() {
        assert_eq!(run_owned(&history("default", 1), 1).outcome, closing());
        assert_eq!(run_owned(&history("shell", 9), 1).outcome, closing());
    }
}
