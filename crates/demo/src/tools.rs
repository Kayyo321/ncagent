/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The simulated tool runner (D45). It runs nothing.
//!
//! Every result is fixed text, with `simulated: true`. A change to a file is described, and no
//! file is touched. A shell command is shown, and no process starts. Nothing here reads a file,
//! lists a directory, or contacts a service (PROJECT.md, O1 point 5).

use harness::tools::{
    ToolErrorKind, find, tool_call_t, tool_failure_t, tool_outcome_t, tool_runner_t,
};
use ncm::backend::cancel_token_t;

use crate::timing::{sleep_checked, timing_t};

/// Runs the registered tools as fixed simulated text.
pub struct demo_tool_runner_t {
    timing: timing_t,
}

impl demo_tool_runner_t {
    pub fn new(timing: timing_t) -> Self {
        demo_tool_runner_t { timing }
    }
}

impl tool_runner_t for demo_tool_runner_t {
    fn run(
        &self,
        call: &tool_call_t,
        cancel: &cancel_token_t,
    ) -> Result<tool_outcome_t, tool_failure_t> {
        let spec = find(&call.tool).ok_or_else(|| {
            failure(
                ToolErrorKind::Invalid,
                format!("unknown tool `{}`", call.tool),
            )
        })?;
        if let Some(arg) = spec
            .args
            .iter()
            .find(|arg| arg.required && call.arg(arg.name).is_none())
        {
            return Err(failure(
                ToolErrorKind::Invalid,
                format!("missing argument `{}`", arg.name),
            ));
        }
        if let Some(message) = not_implemented(spec.name) {
            return Err(failure(ToolErrorKind::NotImplemented, message));
        }
        if !sleep_checked(self.timing.tool_ms, cancel) {
            return Err(failure(
                ToolErrorKind::Cancelled,
                "cancelled while the simulated tool ran",
            ));
        }

        let output = simulated_output(call)?;
        let (output, truncated) = limit_output(&output, spec.result_limit);
        Ok(tool_outcome_t {
            ok: true,
            output,
            truncated,
            simulated: true,
        })
    }
}

/// The value of an argument, or empty when the call lacks it. `run` has already checked the
/// required ones, so empty is only seen for optional arguments.
fn value<'a>(call: &'a tool_call_t, name: &str) -> &'a str {
    call.arg(name).unwrap_or("")
}

fn failure(kind: ToolErrorKind, message: impl Into<String>) -> tool_failure_t {
    tool_failure_t {
        kind,
        message: message.into(),
    }
}

/// Tools that the demo does not implement, with the milestone that will.
fn not_implemented(name: &str) -> Option<&'static str> {
    match name {
        "spawn_agent" | "await_agent" | "cancel_agent" => Some("sub-agents (milestone H4)"),
        "search_history" => Some("search_history (milestone H3)"),
        _ => None,
    }
}

/// The fixed text of a tool that the demo implements. A missing file fails here.
fn simulated_output(call: &tool_call_t) -> Result<String, tool_failure_t> {
    let output = match call.tool.as_str() {
        "list_dir" => list_dir_output(value(call, "path")),
        "read_file" => {
            let path = value(call, "path");
            if path.contains("missing") {
                return Err(failure(
                    ToolErrorKind::Failed,
                    format!("No such file or directory: {path} (simulated)"),
                ));
            }
            read_file_output(path)
        }
        "search" => search_output(value(call, "pattern"), value(call, "path")),
        "replace_lines" => replace_lines_output(
            value(call, "path"),
            value(call, "start"),
            value(call, "end"),
            value(call, "text"),
        ),
        "insert_lines" => insert_lines_output(
            value(call, "path"),
            value(call, "after"),
            value(call, "text"),
        ),
        "delete_lines" => delete_lines_output(
            value(call, "path"),
            value(call, "start"),
            value(call, "end"),
        ),
        "create_file" => format!("SIMULATED: no file was created at {}.", value(call, "path")),
        "append_lines" => append_lines_output(value(call, "path"), value(call, "text")),
        "run_shell" => shell_output(value(call, "command")),
        other => {
            return Err(failure(
                ToolErrorKind::Invalid,
                format!("no simulated result for `{other}`"),
            ));
        }
    };
    Ok(output)
}

fn list_dir_output(path: &str) -> String {
    format!(
        "(simulated listing of `{path}`; the demo reads no directory)\nCargo.toml\nREADME.md\nsrc/"
    )
}

fn read_file_output(path: &str) -> String {
    format!(
        "(simulated content for {path}; the demo reads no file)\nfn main() {{\n    println!(\"simulated\");\n}}"
    )
}

fn search_output(pattern: &str, path: &str) -> String {
    format!(
        "(simulated search for `{pattern}` under {path}; nothing was searched)\nsrc/main.rs:1: // simulated match\nREADME.md:1: // simulated match"
    )
}

fn replace_lines_output(path: &str, start: &str, end: &str, text: &str) -> String {
    let mut out = format!(
        "SIMULATED: no file was changed. Simulated diff of the requested edit:\n--- a/{path}\n+++ b/{path}\n@@ lines {start}-{end} @@\n-(the old lines are not read in the demo)\n"
    );
    push_added_lines(&mut out, text);
    out
}

fn insert_lines_output(path: &str, after: &str, text: &str) -> String {
    let mut out = format!("SIMULATED: no file was changed. Insert after line {after} of {path}:\n");
    push_added_lines(&mut out, text);
    out
}

fn delete_lines_output(path: &str, start: &str, end: &str) -> String {
    format!("SIMULATED: no file was changed. Delete lines {start}-{end} of {path}.")
}

fn append_lines_output(path: &str, text: &str) -> String {
    let mut out = format!("SIMULATED: no file was changed. Append to {path}:\n");
    push_added_lines(&mut out, text);
    out
}

/// One `+` line per line of `text`, each on its own line of `out`.
fn push_added_lines(out: &mut String, text: &str) {
    for line in text.split('\n') {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out.pop();
}

fn shell_output(command: &str) -> String {
    let mut out = format!("$ {command}\n(simulated output; no command was run)");
    if let Some(echoed) = echoed_text(command) {
        out.push('\n');
        out.push_str(echoed);
    }
    out
}

/// The text an `echo` command would print, for display only. Not a shell parser: one pair of
/// matching quotes is removed, and nothing is expanded.
fn echoed_text(command: &str) -> Option<&str> {
    let rest = command.strip_prefix("echo")?;
    if !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    Some(strip_quotes(rest.trim()))
}

fn strip_quotes(text: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    text
}

/// Cuts `text` to at most `limit` bytes, on a character boundary. Returns the text and whether
/// it was cut.
fn limit_output(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_string(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text.get(..end).unwrap_or("").to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::tools::ResultLimit;

    fn call(tool: &str, args: &[(&str, &str)]) -> tool_call_t {
        tool_call_t {
            id: "t1".to_string(),
            tool: tool.to_string(),
            args: args
                .iter()
                .map(|(name, value)| harness::tools::tool_arg_t {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
        }
    }

    fn runner() -> demo_tool_runner_t {
        demo_tool_runner_t::new(timing_t::instant())
    }

    fn run(call: &tool_call_t) -> Result<tool_outcome_t, tool_failure_t> {
        runner().run(call, &cancel_token_t::new())
    }

    /// A value for every argument name of the registry.
    fn sample_value(name: &str) -> &'static str {
        match name {
            "path" => "src/lib.rs",
            "lines" => "1 5",
            "pattern" => "needle",
            "start" => "3",
            "end" => "4",
            "after" => "2",
            "text" => "let x = 1;",
            "command" => "echo hi",
            "role" => "reviewer",
            "task" => "check it",
            "budget_passes" => "4",
            "workspace" => "shared",
            "id" => "a1",
            "query" => "older",
            _ => "",
        }
    }

    #[test]
    fn every_registered_tool_gives_simulated_output_or_a_named_refusal() {
        for spec in harness::tools::registry() {
            let args: Vec<(&str, &str)> = spec
                .args
                .iter()
                .map(|arg| (arg.name, sample_value(arg.name)))
                .collect();
            let result = run(&call(spec.name, &args));
            match not_implemented(spec.name) {
                Some(message) => {
                    let err = result.expect_err("a refused tool must return an error");
                    assert_eq!(err.kind, ToolErrorKind::NotImplemented);
                    assert_eq!(err.message, message);
                }
                None => {
                    let outcome = result.unwrap_or_else(|err| panic!("{}: {err:?}", spec.name));
                    assert!(outcome.ok);
                    assert!(outcome.simulated);
                    assert!(!outcome.truncated);
                    assert!(
                        outcome.output.to_lowercase().contains("simulated"),
                        "{} output must say it is simulated",
                        spec.name
                    );
                }
            }
        }
    }

    #[test]
    fn sub_agents_and_history_name_their_milestones() {
        let spawn_args = [
            ("role", "reviewer"),
            ("task", "check the diff"),
            ("budget_passes", "4"),
            ("workspace", "shared"),
        ];
        let spawn = run(&call("spawn_agent", &spawn_args)).expect_err("not implemented");
        assert_eq!(spawn.kind, ToolErrorKind::NotImplemented);
        for name in ["await_agent", "cancel_agent"] {
            let err =
                run(&call(name, &[("id", "a1")])).expect_err("sub-agents are not implemented");
            assert_eq!(err.kind, ToolErrorKind::NotImplemented);
            assert_eq!(err.message, "sub-agents (milestone H4)");
        }
        assert_eq!(spawn.message, "sub-agents (milestone H4)");
        let err = run(&call("search_history", &[("query", "x")]))
            .expect_err("history search is not implemented");
        assert_eq!(err.message, "search_history (milestone H3)");
    }

    #[test]
    fn an_unknown_tool_is_invalid() {
        let err = run(&call("rm_everything", &[])).expect_err("unknown tool");
        assert_eq!(err.kind, ToolErrorKind::Invalid);
    }

    #[test]
    fn a_missing_required_argument_is_invalid() {
        let err = run(&call("read_file", &[])).expect_err("path is required");
        assert_eq!(err.kind, ToolErrorKind::Invalid);
        assert_eq!(err.message, "missing argument `path`");
    }

    #[test]
    fn read_file_returns_a_simulated_excerpt() {
        let outcome = run(&call(
            "read_file",
            &[("path", "src/main.rs"), ("lines", "1 20")],
        ))
        .expect("read_file succeeds");
        assert!(
            outcome
                .output
                .starts_with("(simulated content for src/main.rs")
        );
        assert!(outcome.simulated);
    }

    #[test]
    fn a_missing_file_fails_with_a_simulated_message() {
        let err = run(&call("read_file", &[("path", "docs/missing.txt")]))
            .expect_err("missing files fail");
        assert_eq!(err.kind, ToolErrorKind::Failed);
        assert_eq!(
            err.message,
            "No such file or directory: docs/missing.txt (simulated)"
        );
    }

    #[test]
    fn list_dir_is_a_fixed_listing() {
        let outcome = run(&call("list_dir", &[("path", ".")])).expect("list_dir succeeds");
        for entry in ["Cargo.toml", "README.md", "src/"] {
            assert!(outcome.output.contains(entry), "missing {entry}");
        }
    }

    #[test]
    fn replace_lines_describes_the_change_and_writes_nothing() {
        let outcome = run(&call(
            "replace_lines",
            &[
                ("path", "src/lib.rs"),
                ("start", "3"),
                ("end", "4"),
                ("text", "// demo edit\nlet x = 1;"),
            ],
        ))
        .expect("replace_lines succeeds");
        assert!(outcome.output.contains("SIMULATED: no file was changed"));
        assert!(outcome.output.contains("--- a/src/lib.rs"));
        assert!(outcome.output.contains("@@ lines 3-4 @@"));
        assert!(outcome.output.contains("+// demo edit\n"));
        assert!(outcome.output.contains("+let x = 1;"));
    }

    #[test]
    fn run_shell_shows_the_command_and_never_runs_it() {
        let outcome = run(&call(
            "run_shell",
            &[("command", "echo \"hello from the demo\"")],
        ))
        .expect("run_shell succeeds");
        assert_eq!(
            outcome.output,
            "$ echo \"hello from the demo\"\n(simulated output; no command was run)\nhello from the demo"
        );

        let cargo = run(&call("run_shell", &[("command", "cargo test")])).expect("shows it");
        assert_eq!(
            cargo.output,
            "$ cargo test\n(simulated output; no command was run)"
        );
    }

    #[test]
    fn echoed_text_removes_one_pair_of_quotes() {
        assert_eq!(echoed_text("echo \"a b\""), Some("a b"));
        assert_eq!(echoed_text("echo 'a b'"), Some("a b"));
        assert_eq!(echoed_text("echo a b"), Some("a b"));
        assert_eq!(echoed_text("echo"), Some(""));
        assert_eq!(echoed_text("echo \""), Some("\""));
        assert_eq!(echoed_text("echos x"), None);
        assert_eq!(echoed_text("cargo test"), None);
    }

    #[test]
    fn a_cancelled_token_stops_the_tool_before_it_runs() {
        let cancel = cancel_token_t::new();
        cancel.cancel();
        let err = runner()
            .run(&call("list_dir", &[("path", ".")]), &cancel)
            .expect_err("cancelled");
        assert_eq!(err.kind, ToolErrorKind::Cancelled);
    }

    #[test]
    fn limit_output_cuts_on_a_character_boundary() {
        let (short, cut) = limit_output("abc", 10);
        assert_eq!(short, "abc");
        assert!(!cut);

        let long = "a".repeat(3000);
        let (text, cut) = limit_output(&long, ResultLimit);
        assert!(cut);
        assert_eq!(text.len(), ResultLimit);

        let accented = "é".repeat(2000);
        let (text, cut) = limit_output(&accented, 2047);
        assert!(cut);
        assert!(text.len() <= 2047);
        assert!(text.chars().all(|c| c == 'é'));
    }
}
