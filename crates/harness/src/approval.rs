/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Approval modes, decisions and the policy that turns a tool call into a verdict (D15).
//!
//! The vocabulary is here, and so is `approval_policy_t`, which holds the session grants and
//! evaluates a call. Paths are checked lexically: nothing here touches the filesystem, so a
//! symbolic link inside the working directory that points outside it is not detected.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::tools::{ApprovalClass, tool_call_t, tool_spec_t};

/// Longest summary, in bytes, before it is cut (`summary_for`).
pub const SummaryLimit: usize = 1200;

/// `ask` (the default), `edit` or `full`. `full` must be selected on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Ask,
    Edit,
    Full,
}

impl ApprovalMode {
    pub fn name(self) -> &'static str {
        match self {
            ApprovalMode::Ask => "ask",
            ApprovalMode::Edit => "edit",
            ApprovalMode::Full => "full",
        }
    }
}

/// The user's answer to an approval request: once, for the rest of the session, or deny.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Once,
    Session,
    Deny,
}

/// What allowed or refused a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSource {
    /// The user answered an approval request.
    User,
    /// The approval mode allows this class without asking.
    Mode,
    /// An earlier "for the session" answer covers this call.
    SessionGrant,
}

/// The policy's verdict on one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow {
        source: ApprovalSource,
    },
    /// The user must decide. `reason` says why, in one line.
    Ask {
        reason: String,
    },
}

/// The approval policy (D15): the mode, the working directory and the session grants.
///
/// A grant is keyed by the tool name and a scope: the exact `path` argument when the call has
/// one, otherwise the first word of a shell command. Only `ApprovalDecision::Session` stores one.
#[derive(Clone, Debug)]
pub struct approval_policy_t {
    mode: ApprovalMode,
    cwd: PathBuf,
    grants: HashSet<(String, String)>,
}

impl approval_policy_t {
    pub fn new(mode: ApprovalMode, cwd: PathBuf) -> Self {
        approval_policy_t {
            mode,
            cwd,
            grants: HashSet::new(),
        }
    }

    /// Sets the mode. Refusing `full` when it was not given on the command line is the caller's
    /// job (`api::session_t::set_approval_mode`).
    pub fn set_mode(&mut self, mode: ApprovalMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> ApprovalMode {
        self.mode
    }

    /// The verdict on one call. A matching session grant wins; otherwise the mode and the class
    /// decide, and in `full` any `path` argument outside the working directory asks.
    pub fn evaluate(&self, spec: &tool_spec_t, call: &tool_call_t) -> Verdict {
        let mode = self.mode.name();
        let class = spec.class.name();
        if spec.name != call.tool {
            return Verdict::Ask {
                reason: format!("{mode} mode, {class} call: the call does not name its tool"),
            };
        }
        if self.grants.contains(&grant_key(call)) {
            return Verdict::Allow {
                source: ApprovalSource::SessionGrant,
            };
        }
        let allow = || Verdict::Allow {
            source: ApprovalSource::Mode,
        };
        let ask = |cause: &str| Verdict::Ask {
            reason: format!("{mode} mode, {class} call: {cause}"),
        };
        let path = call.arg("path");
        let inside = path.is_some_and(|given| path_inside_cwd(&self.cwd, given));
        match self.mode {
            ApprovalMode::Full => {
                if path.is_some() && !inside {
                    ask("the path is outside the working directory")
                } else {
                    allow()
                }
            }
            ApprovalMode::Edit => match spec.class {
                ApprovalClass::Read => allow(),
                ApprovalClass::Write if inside => allow(),
                ApprovalClass::Write => {
                    ask("needs approval, the path is missing or outside the working directory")
                }
                ApprovalClass::Shell | ApprovalClass::Agent => ask("needs approval"),
            },
            ApprovalMode::Ask => match spec.class {
                ApprovalClass::Read => allow(),
                ApprovalClass::Write | ApprovalClass::Shell | ApprovalClass::Agent => {
                    ask("needs approval")
                }
            },
        }
    }

    /// Stores a grant when the user chose "for the session". `Once` and `Deny` store nothing.
    pub fn record(&mut self, decision: ApprovalDecision, call: &tool_call_t) {
        if decision == ApprovalDecision::Session {
            self.grants.insert(grant_key(call));
        }
    }
}

fn grant_key(call: &tool_call_t) -> (String, String) {
    (call.tool.clone(), grant_scope(call))
}

fn grant_scope(call: &tool_call_t) -> String {
    if let Some(path) = call.arg("path") {
        return path.to_string();
    }
    if call.tool == "run_shell" {
        return call
            .arg("command")
            .and_then(|command| command.split_whitespace().next())
            .unwrap_or("")
            .to_string();
    }
    String::new()
}

/// True when `path` names a place inside `cwd`. The check is lexical: relative paths resolve
/// against `cwd`, `.` and `..` are normalised, and an absolute path is inside only when `cwd` is
/// a component-wise prefix of it. An empty path, or one with a NUL, is never inside.
pub fn path_inside_cwd(cwd: &Path, path: &str) -> bool {
    if path.is_empty() || path.contains('\0') {
        return false;
    }
    let joined = cwd.join(path);
    match (lexically_normal(cwd), lexically_normal(&joined)) {
        (Some(base), Some(target)) => target.starts_with(&base),
        _ => false,
    }
}

/// Removes `.` and resolves `..` without touching the filesystem. `None` when a relative path
/// climbs above its own start, because such a path cannot be placed.
fn lexically_normal(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => out.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if !out.has_root() {
                    return None;
                }
            }
            Component::Normal(name) => {
                out.push(name);
                depth += 1;
            }
        }
    }
    Some(out)
}

/// The text an approval request shows. `run_shell` gets the exact command. A write tool gets a
/// plan: `<tool> <path> [range]`, then one `+ ` line for each line of `text`. Any other tool gets
/// `<tool> key=value ...`. The result is cut at `SummaryLimit` bytes.
pub fn summary_for(call: &tool_call_t) -> String {
    let full = if call.tool == "run_shell" {
        call.arg("command").unwrap_or("").to_string()
    } else if is_write_tool(&call.tool) {
        write_plan(call)
    } else {
        key_value_line(call)
    };
    cut_summary(full)
}

fn is_write_tool(tool: &str) -> bool {
    crate::tools::find(tool).is_some_and(|spec| spec.class == ApprovalClass::Write)
}

fn write_plan(call: &tool_call_t) -> String {
    let mut head = vec![call.tool.clone()];
    if let Some(path) = call.arg("path") {
        head.push(display_value(path));
    }
    match (call.arg("start"), call.arg("end"), call.arg("after")) {
        (Some(start), Some(end), _) => {
            head.push(format!("{}-{}", display_value(start), display_value(end)));
        }
        (_, _, Some(after)) => head.push(format!("after {}", display_value(after))),
        _ => {}
    }
    let mut lines = vec![head.join(" ")];
    if let Some(text) = call.arg("text") {
        lines.extend(
            text.lines()
                .map(|line| format!("+ {}", display_value(line))),
        );
    }
    lines.join("\n")
}

fn key_value_line(call: &tool_call_t) -> String {
    let mut parts = vec![call.tool.clone()];
    parts.extend(
        call.args
            .iter()
            .map(|arg| format!("{}={}", display_value(&arg.name), display_value(&arg.value))),
    );
    parts.join(" ")
}

/// Makes a value safe for one line: newlines and carriage returns are written as escapes, and
/// other control characters (except tab) become `?`.
fn display_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push('\t'),
            c if c.is_control() => out.push('?'),
            c => out.push(c),
        }
    }
    out
}

fn cut_summary(text: String) -> String {
    if text.len() <= SummaryLimit {
        return text;
    }
    let mut end = SummaryLimit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n... (cut)", text.get(..end).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{find, tool_arg_t};

    fn spec(name: &str) -> &'static tool_spec_t {
        match find(name) {
            Some(spec) => spec,
            None => panic!("no tool named {name}"),
        }
    }

    fn call(tool: &str, args: &[(&str, &str)]) -> tool_call_t {
        tool_call_t {
            id: "call_1".to_string(),
            tool: tool.to_string(),
            args: args
                .iter()
                .map(|(name, value)| tool_arg_t {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
        }
    }

    fn policy(mode: ApprovalMode) -> approval_policy_t {
        approval_policy_t::new(mode, PathBuf::from("/work/dir"))
    }

    fn ask(reason: &str) -> Verdict {
        Verdict::Ask {
            reason: reason.to_string(),
        }
    }

    fn allow_mode() -> Verdict {
        Verdict::Allow {
            source: ApprovalSource::Mode,
        }
    }

    #[test]
    fn ask_mode_runs_reads_and_asks_for_the_rest() {
        let p = policy(ApprovalMode::Ask);
        let read = call("read_file", &[("path", "src/main.rs")]);
        assert_eq!(p.evaluate(spec("read_file"), &read), allow_mode());
        let write = call("create_file", &[("path", "src/new.rs")]);
        assert_eq!(
            p.evaluate(spec("create_file"), &write),
            ask("ask mode, write call: needs approval")
        );
        let shell = call("run_shell", &[("command", "ls")]);
        assert_eq!(
            p.evaluate(spec("run_shell"), &shell),
            ask("ask mode, shell call: needs approval")
        );
        let agent = call(
            "spawn_agent",
            &[
                ("role", "r"),
                ("task", "t"),
                ("budget_passes", "1"),
                ("workspace", "w"),
            ],
        );
        assert_eq!(
            p.evaluate(spec("spawn_agent"), &agent),
            ask("ask mode, agent call: needs approval")
        );
    }

    #[test]
    fn edit_mode_runs_inside_writes_and_asks_outside() {
        let p = policy(ApprovalMode::Edit);
        let inside = call("create_file", &[("path", "src/new.rs")]);
        assert_eq!(p.evaluate(spec("create_file"), &inside), allow_mode());
        let outside = call("create_file", &[("path", "../other/new.rs")]);
        assert_eq!(
            p.evaluate(spec("create_file"), &outside),
            ask(
                "edit mode, write call: needs approval, the path is missing or outside the working directory"
            )
        );
        let missing = call("append_lines", &[("text", "x")]);
        assert!(matches!(
            p.evaluate(spec("append_lines"), &missing),
            Verdict::Ask { .. }
        ));
        let read_outside = call("read_file", &[("path", "/etc/hosts")]);
        assert_eq!(p.evaluate(spec("read_file"), &read_outside), allow_mode());
    }

    #[test]
    fn edit_mode_asks_for_shell_and_agent() {
        let p = policy(ApprovalMode::Edit);
        let shell = call("run_shell", &[("command", "cargo test")]);
        assert_eq!(
            p.evaluate(spec("run_shell"), &shell),
            ask("edit mode, shell call: needs approval")
        );
        let agent = call(
            "spawn_agent",
            &[
                ("role", "r"),
                ("task", "t"),
                ("budget_passes", "1"),
                ("workspace", "w"),
            ],
        );
        assert_eq!(
            p.evaluate(spec("spawn_agent"), &agent),
            ask("edit mode, agent call: needs approval")
        );
    }

    #[test]
    fn full_mode_asks_only_for_paths_outside_cwd() {
        let p = policy(ApprovalMode::Full);
        let shell = call("run_shell", &[("command", "rm -rf build")]);
        assert_eq!(p.evaluate(spec("run_shell"), &shell), allow_mode());
        let inside = call(
            "replace_lines",
            &[
                ("path", "src/lib.rs"),
                ("start", "1"),
                ("end", "2"),
                ("text", "x"),
            ],
        );
        assert_eq!(p.evaluate(spec("replace_lines"), &inside), allow_mode());
        let outside_write = call("create_file", &[("path", "/tmp/x")]);
        assert_eq!(
            p.evaluate(spec("create_file"), &outside_write),
            ask("full mode, write call: the path is outside the working directory")
        );
        let outside_read = call("search", &[("pattern", "a"), ("path", "/etc")]);
        assert_eq!(
            p.evaluate(spec("search"), &outside_read),
            ask("full mode, read call: the path is outside the working directory")
        );
    }

    #[test]
    fn session_grant_covers_the_same_path_only() {
        let mut p = policy(ApprovalMode::Ask);
        let first = call("create_file", &[("path", "src/a.rs")]);
        p.record(ApprovalDecision::Session, &first);
        assert_eq!(
            p.evaluate(spec("create_file"), &first),
            Verdict::Allow {
                source: ApprovalSource::SessionGrant
            }
        );
        let other = call("create_file", &[("path", "src/b.rs")]);
        assert!(matches!(
            p.evaluate(spec("create_file"), &other),
            Verdict::Ask { .. }
        ));
    }

    #[test]
    fn session_grant_for_shell_uses_the_first_word() {
        let mut p = policy(ApprovalMode::Ask);
        let status = call("run_shell", &[("command", "git status --short")]);
        p.record(ApprovalDecision::Session, &status);
        let same_word = call("run_shell", &[("command", "git   log")]);
        assert_eq!(
            p.evaluate(spec("run_shell"), &same_word),
            Verdict::Allow {
                source: ApprovalSource::SessionGrant
            }
        );
        let other_word = call("run_shell", &[("command", "cargo test")]);
        assert!(matches!(
            p.evaluate(spec("run_shell"), &other_word),
            Verdict::Ask { .. }
        ));
    }

    #[test]
    fn once_and_deny_store_nothing() {
        let mut p = policy(ApprovalMode::Ask);
        let write = call("create_file", &[("path", "src/a.rs")]);
        p.record(ApprovalDecision::Once, &write);
        p.record(ApprovalDecision::Deny, &write);
        assert!(matches!(
            p.evaluate(spec("create_file"), &write),
            Verdict::Ask { .. }
        ));
    }

    #[test]
    fn a_grant_wins_over_the_outside_path_rule() {
        let mut p = policy(ApprovalMode::Full);
        let outside = call("create_file", &[("path", "/tmp/x")]);
        p.record(ApprovalDecision::Session, &outside);
        assert_eq!(
            p.evaluate(spec("create_file"), &outside),
            Verdict::Allow {
                source: ApprovalSource::SessionGrant
            }
        );
    }

    #[test]
    fn set_mode_changes_the_verdicts() {
        let mut p = policy(ApprovalMode::Ask);
        let shell = call("run_shell", &[("command", "ls")]);
        assert!(matches!(
            p.evaluate(spec("run_shell"), &shell),
            Verdict::Ask { .. }
        ));
        p.set_mode(ApprovalMode::Full);
        assert_eq!(p.mode(), ApprovalMode::Full);
        assert_eq!(p.evaluate(spec("run_shell"), &shell), allow_mode());
    }

    #[test]
    fn a_call_that_names_another_tool_asks() {
        let p = policy(ApprovalMode::Full);
        let mismatched = call("run_shell", &[("command", "ls")]);
        assert_eq!(
            p.evaluate(spec("list_dir"), &mismatched),
            ask("full mode, read call: the call does not name its tool")
        );
    }

    #[test]
    fn path_inside_cwd_resolves_relative_and_absolute_paths() {
        let cwd = Path::new("/work/dir");
        assert!(path_inside_cwd(cwd, "src/main.rs"));
        assert!(path_inside_cwd(cwd, "./src/../lib.rs"));
        assert!(path_inside_cwd(cwd, "."));
        assert!(path_inside_cwd(cwd, "/work/dir/src/x"));
        assert!(!path_inside_cwd(cwd, "../x"));
        assert!(!path_inside_cwd(cwd, "src/../../x"));
        assert!(!path_inside_cwd(cwd, "/work/dir/../x"));
        assert!(!path_inside_cwd(cwd, "/etc/passwd"));
    }

    #[test]
    fn path_inside_cwd_compares_whole_components() {
        let cwd = Path::new("/work/dir");
        assert!(!path_inside_cwd(cwd, "/work/dir2"));
        assert!(!path_inside_cwd(cwd, "/work/dir2/file"));
        assert!(!path_inside_cwd(Path::new("/work/dir"), "/work"));
    }

    #[test]
    fn path_inside_cwd_rejects_empty_nul_and_unplaceable_paths() {
        let cwd = Path::new("/work/dir");
        assert!(!path_inside_cwd(cwd, ""));
        assert!(!path_inside_cwd(cwd, "src/\0x"));
        assert!(!path_inside_cwd(Path::new(""), "../x"));
        assert!(path_inside_cwd(Path::new(""), "x/y"));
    }

    #[test]
    fn path_inside_cwd_handles_a_root_cwd() {
        assert!(path_inside_cwd(Path::new("/"), "/anything"));
        assert!(path_inside_cwd(Path::new("/"), "../../x"));
    }

    #[test]
    fn summary_for_shell_is_the_exact_command() {
        let command = "printf 'a\\n'\nls -la";
        let shell = call("run_shell", &[("command", command)]);
        assert_eq!(summary_for(&shell), command);
    }

    #[test]
    fn summary_for_write_is_a_plan_with_one_line_per_text_line() {
        let replace = call(
            "replace_lines",
            &[
                ("path", "src/main.rs"),
                ("start", "3"),
                ("end", "5"),
                ("text", "one\ntwo"),
            ],
        );
        assert_eq!(
            summary_for(&replace),
            "replace_lines src/main.rs 3-5\n+ one\n+ two"
        );
        let insert = call(
            "insert_lines",
            &[("path", "src/a.rs"), ("after", "3"), ("text", "x")],
        );
        assert_eq!(summary_for(&insert), "insert_lines src/a.rs after 3\n+ x");
        let create = call("create_file", &[("path", "src/new.rs")]);
        assert_eq!(summary_for(&create), "create_file src/new.rs");
    }

    #[test]
    fn summary_for_other_tools_is_key_value() {
        let read = call("read_file", &[("path", "src/main.rs"), ("lines", "1 80")]);
        assert_eq!(summary_for(&read), "read_file path=src/main.rs lines=1 80");
        let search = call("search", &[("pattern", "a\nb"), ("path", ".")]);
        assert_eq!(summary_for(&search), "search pattern=a\\nb path=.");
    }

    #[test]
    fn summary_for_keeps_a_plan_line_free_of_control_characters() {
        let write = call("create_file", &[("path", "bad\u{1b}[2J\rname")]);
        assert_eq!(summary_for(&write), "create_file bad?[2J\\rname");
    }

    #[test]
    fn summary_is_cut_on_a_char_boundary() {
        let long = format!("a{}", "é".repeat(700));
        let shell = call("run_shell", &[("command", &long)]);
        let summary = summary_for(&shell);
        let expected_prefix = format!("a{}", "é".repeat(599));
        assert_eq!(summary, format!("{expected_prefix}\n... (cut)"));
        assert!(expected_prefix.len() <= SummaryLimit);
    }

    #[test]
    fn short_summary_is_not_cut() {
        let shell = call("run_shell", &[("command", "ls")]);
        assert_eq!(summary_for(&shell), "ls");
    }
}
