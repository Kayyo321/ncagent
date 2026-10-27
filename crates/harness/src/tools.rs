/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! The tool registry (D35) and the tool-runner interface.
//!
//! The registry is the only list of tools. The reply parser, the approval policy and every
//! runner read it. P1 declares the tools of PROJECT.md, "Tools", except the `term_*` family
//! (milestone U2). Declaring a tool here does not implement it: a runner does that, and the only
//! runner in P1 is the simulated one in `crates/demo` (D45).

use ncm::backend::cancel_token_t;
use serde::{Deserialize, Serialize};

/// The approval class of a tool (D15).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalClass {
    Read,
    Write,
    Shell,
    Agent,
}

impl ApprovalClass {
    pub fn name(self) -> &'static str {
        match self {
            ApprovalClass::Read => "read",
            ApprovalClass::Write => "write",
            ApprovalClass::Shell => "shell",
            ApprovalClass::Agent => "agent",
        }
    }
}

/// One argument of a tool. Names are exact and snake_case. A longer value makes a call invalid.
#[derive(Debug)]
pub struct arg_spec_t {
    pub name: &'static str,
    pub required: bool,
    /// Longest value, in bytes.
    pub max_len: usize,
}

/// One tool.
#[derive(Debug)]
pub struct tool_spec_t {
    pub name: &'static str,
    pub class: ApprovalClass,
    pub args: &'static [arg_spec_t],
    /// Longest result, in bytes. A runner cuts a longer result and says so.
    pub result_limit: usize,
    pub summary: &'static str,
}

/// Result limit of PROJECT.md, "Tools".
pub const ResultLimit: usize = 2048;

const PathArg: arg_spec_t = arg_spec_t {
    name: "path",
    required: true,
    max_len: 256,
};
const LinesArg: arg_spec_t = arg_spec_t {
    name: "lines",
    required: false,
    max_len: 24,
};
const PatternArg: arg_spec_t = arg_spec_t {
    name: "pattern",
    required: true,
    max_len: 100,
};
const StartArg: arg_spec_t = arg_spec_t {
    name: "start",
    required: true,
    max_len: 8,
};
const EndArg: arg_spec_t = arg_spec_t {
    name: "end",
    required: true,
    max_len: 8,
};
const AfterArg: arg_spec_t = arg_spec_t {
    name: "after",
    required: true,
    max_len: 8,
};
const TextArg: arg_spec_t = arg_spec_t {
    name: "text",
    required: true,
    max_len: 400,
};
const CommandArg: arg_spec_t = arg_spec_t {
    name: "command",
    required: true,
    max_len: 200,
};
const RoleArg: arg_spec_t = arg_spec_t {
    name: "role",
    required: true,
    max_len: 32,
};
const TaskArg: arg_spec_t = arg_spec_t {
    name: "task",
    required: true,
    max_len: 300,
};
const BudgetArg: arg_spec_t = arg_spec_t {
    name: "budget_passes",
    required: true,
    max_len: 8,
};
const WorkspaceArg: arg_spec_t = arg_spec_t {
    name: "workspace",
    required: true,
    max_len: 16,
};
const IdArg: arg_spec_t = arg_spec_t {
    name: "id",
    required: true,
    max_len: 32,
};
const QueryArg: arg_spec_t = arg_spec_t {
    name: "query",
    required: true,
    max_len: 100,
};

static Registry: [tool_spec_t; 13] = [
    tool_spec_t {
        name: "list_dir",
        class: ApprovalClass::Read,
        args: &[PathArg],
        result_limit: ResultLimit,
        summary: "Entries of one directory.",
    },
    tool_spec_t {
        name: "read_file",
        class: ApprovalClass::Read,
        args: &[PathArg, LinesArg],
        result_limit: ResultLimit,
        summary: "Lines `a b`, or the whole file.",
    },
    tool_spec_t {
        name: "search",
        class: ApprovalClass::Read,
        args: &[PatternArg, PathArg],
        result_limit: ResultLimit,
        summary: "Fixed-string search.",
    },
    tool_spec_t {
        name: "replace_lines",
        class: ApprovalClass::Write,
        args: &[PathArg, StartArg, EndArg, TextArg],
        result_limit: ResultLimit,
        summary: "Replace a range of lines. Shows a diff.",
    },
    tool_spec_t {
        name: "insert_lines",
        class: ApprovalClass::Write,
        args: &[PathArg, AfterArg, TextArg],
        result_limit: ResultLimit,
        summary: "Insert lines after a line.",
    },
    tool_spec_t {
        name: "delete_lines",
        class: ApprovalClass::Write,
        args: &[PathArg, StartArg, EndArg],
        result_limit: ResultLimit,
        summary: "Delete a range of lines.",
    },
    tool_spec_t {
        name: "create_file",
        class: ApprovalClass::Write,
        args: &[PathArg],
        result_limit: ResultLimit,
        summary: "Create an empty file. Fails if it exists.",
    },
    tool_spec_t {
        name: "append_lines",
        class: ApprovalClass::Write,
        args: &[PathArg, TextArg],
        result_limit: ResultLimit,
        summary: "Append lines, to build a file in several calls.",
    },
    tool_spec_t {
        name: "run_shell",
        class: ApprovalClass::Shell,
        args: &[CommandArg],
        result_limit: ResultLimit,
        summary: "Run a shell command.",
    },
    tool_spec_t {
        name: "spawn_agent",
        class: ApprovalClass::Agent,
        args: &[RoleArg, TaskArg, BudgetArg, WorkspaceArg],
        result_limit: ResultLimit,
        summary: "Start a sub-agent.",
    },
    tool_spec_t {
        name: "await_agent",
        class: ApprovalClass::Read,
        args: &[IdArg],
        result_limit: ResultLimit,
        summary: "Wait for a sub-agent's result.",
    },
    tool_spec_t {
        name: "cancel_agent",
        class: ApprovalClass::Read,
        args: &[IdArg],
        result_limit: ResultLimit,
        summary: "Cancel a sub-agent.",
    },
    tool_spec_t {
        name: "search_history",
        class: ApprovalClass::Read,
        args: &[QueryArg],
        result_limit: ResultLimit,
        summary: "Search the event log for older turns.",
    },
];

/// Every registered tool, in a fixed order.
pub fn registry() -> &'static [tool_spec_t] {
    &Registry
}

/// The tool with this exact name.
pub fn find(name: &str) -> Option<&'static tool_spec_t> {
    Registry.iter().find(|spec| spec.name == name)
}

/// One `key: value` argument of a call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct tool_arg_t {
    pub name: String,
    pub value: String,
}

/// A call as the reply parser reads it, before the controller gives it an id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct parsed_call_t {
    pub tool: String,
    pub args: Vec<tool_arg_t>,
}

/// A call the controller runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct tool_call_t {
    pub id: String,
    pub tool: String,
    pub args: Vec<tool_arg_t>,
}

impl tool_call_t {
    /// The value of an argument, when present.
    pub fn arg(&self, name: &str) -> Option<&str> {
        self.args
            .iter()
            .find(|arg| arg.name == name)
            .map(|arg| arg.value.as_str())
    }
}

/// Why a tool did not produce a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorKind {
    /// The user, or the policy, refused the call.
    Denied,
    /// The tool ran and failed.
    Failed,
    /// The call was cancelled while it ran.
    Cancelled,
    /// The call did not match the registry.
    Invalid,
    /// No runner implements this tool.
    NotImplemented,
}

impl ToolErrorKind {
    pub fn name(self) -> &'static str {
        match self {
            ToolErrorKind::Denied => "denied",
            ToolErrorKind::Failed => "failed",
            ToolErrorKind::Cancelled => "cancelled",
            ToolErrorKind::Invalid => "invalid",
            ToolErrorKind::NotImplemented => "not_implemented",
        }
    }
}

/// A tool that could not produce a result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct tool_failure_t {
    pub kind: ToolErrorKind,
    pub message: String,
}

/// A tool's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct tool_outcome_t {
    /// False when the tool ran and reported failure, such as a non-zero exit.
    pub ok: bool,
    pub output: String,
    /// The runner cut `output` at the result limit.
    pub truncated: bool,
    /// The output is fixed demo text and nothing ran.
    pub simulated: bool,
}

/// Runs registered tools. The runner checks the token between operations.
pub trait tool_runner_t: Send + Sync {
    fn run(
        &self,
        call: &tool_call_t,
        cancel: &cancel_token_t,
    ) -> Result<tool_outcome_t, tool_failure_t>;
}

/// The runner a production session gets until real tool execution exists (milestone H2).
/// Every call fails with a named error, and nothing runs.
pub struct unavailable_tool_runner_t;

impl tool_runner_t for unavailable_tool_runner_t {
    fn run(
        &self,
        call: &tool_call_t,
        _cancel: &cancel_token_t,
    ) -> Result<tool_outcome_t, tool_failure_t> {
        Err(tool_failure_t {
            kind: ToolErrorKind::NotImplemented,
            message: format!(
                "tool execution is not implemented (milestone H2); `{}` did not run",
                call.tool
            ),
        })
    }
}
