/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Slash commands: a small, fixed set typed in the composer.

use harness::approval::ApprovalMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct command_t {
    pub name: &'static str,
    pub args: &'static str,
    pub about: &'static str,
}

pub const Commands: [command_t; 10] = [
    command_t {
        name: "/help",
        args: "",
        about: "keys, mouse and commands",
    },
    command_t {
        name: "/new",
        args: "",
        about: "start a new chat",
    },
    command_t {
        name: "/sidebar",
        args: "",
        about: "show or hide the sidebar",
    },
    command_t {
        name: "/approval",
        args: "ask|edit",
        about: "set the approval mode",
    },
    command_t {
        name: "/cancel",
        args: "",
        about: "stop the turn after the current pass",
    },
    command_t {
        name: "/clear-queue",
        args: "",
        about: "drop queued messages",
    },
    command_t {
        name: "/copy",
        args: "",
        about: "copy the last reply (OSC 52)",
    },
    command_t {
        name: "/animate",
        args: "",
        about: "turn the generation effect on or off",
    },
    command_t {
        name: "/folder",
        args: "[name]",
        about: "new folder at the top level",
    },
    command_t {
        name: "/quit",
        args: "",
        about: "leave ncagent",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    New,
    Sidebar,
    Approval(ApprovalMode),
    Cancel,
    ClearQueue,
    Copy,
    Animate,
    /// A new top-level folder: with a name it is created at once, without one the prompt opens.
    Folder(Option<String>),
    Quit,
}

/// Parse a composer line. `None` when it is not a command; `Some(Err)` names the problem.
pub fn parse(text: &str) -> Option<Result<Command, String>> {
    let text = text.trim();
    if !text.starts_with('/') || text.starts_with("//") {
        return None;
    }
    let mut parts = text.split_whitespace();
    let name = parts.next().unwrap_or("");
    let arg = parts.next();
    let extra = parts.next();
    let simple = |cmd: Command| -> Result<Command, String> {
        match arg {
            None => Ok(cmd),
            Some(_) => Err(format!("{name} takes no argument")),
        }
    };
    let result = match name {
        "/help" => simple(Command::Help),
        "/new" => simple(Command::New),
        "/sidebar" => simple(Command::Sidebar),
        "/cancel" => simple(Command::Cancel),
        "/clear-queue" => simple(Command::ClearQueue),
        "/copy" => simple(Command::Copy),
        "/animate" => simple(Command::Animate),
        "/quit" | "/exit" => simple(Command::Quit),
        "/folder" => {
            let rest = text.get(name.len()..).unwrap_or("").trim();
            Ok(Command::Folder(
                (!rest.is_empty()).then(|| rest.to_string()),
            ))
        }
        "/approval" => match (arg, extra) {
            (Some("ask"), None) => Ok(Command::Approval(ApprovalMode::Ask)),
            (Some("edit"), None) => Ok(Command::Approval(ApprovalMode::Edit)),
            (Some("full"), None) => Ok(Command::Approval(ApprovalMode::Full)),
            _ => Err("usage: /approval ask|edit".to_string()),
        },
        other => Err(format!(
            "unknown command {}; /help lists them",
            crate::text::truncate(&crate::text::sanitize(other, false), 24)
        )),
    };
    Some(result)
}

/// Commands whose name starts with the typed word. Empty when the text is not a bare command
/// word (it has a space or a newline).
pub fn completions(text: &str) -> Vec<command_t> {
    if !text.starts_with('/') || text.contains(char::is_whitespace) {
        return Vec::new();
    }
    Commands
        .iter()
        .filter(|c| c.name.starts_with(text))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_command() {
        assert_eq!(parse("/help"), Some(Ok(Command::Help)));
        assert_eq!(parse(" /new "), Some(Ok(Command::New)));
        assert_eq!(parse("/sidebar"), Some(Ok(Command::Sidebar)));
        assert_eq!(
            parse("/approval edit"),
            Some(Ok(Command::Approval(ApprovalMode::Edit)))
        );
        assert_eq!(
            parse("/approval full"),
            Some(Ok(Command::Approval(ApprovalMode::Full)))
        );
        assert_eq!(parse("/cancel"), Some(Ok(Command::Cancel)));
        assert_eq!(parse("/clear-queue"), Some(Ok(Command::ClearQueue)));
        assert_eq!(parse("/copy"), Some(Ok(Command::Copy)));
        assert_eq!(parse("/animate"), Some(Ok(Command::Animate)));
        assert_eq!(parse("/quit"), Some(Ok(Command::Quit)));
        assert_eq!(parse("/folder"), Some(Ok(Command::Folder(None))));
        assert_eq!(
            parse("/folder  Research notes "),
            Some(Ok(Command::Folder(Some("Research notes".into()))))
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse("//not a command"), None);
        assert!(matches!(parse("/approval"), Some(Err(_))));
        assert!(matches!(parse("/approval maybe"), Some(Err(_))));
        assert!(matches!(parse("/help me"), Some(Err(_))));
        assert!(matches!(parse("/bogus"), Some(Err(_))));
    }

    #[test]
    fn completion_by_prefix() {
        let names: Vec<_> = completions("/c").iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["/cancel", "/clear-queue", "/copy"]);
        assert_eq!(completions("/").len(), Commands.len());
        assert!(completions("/help x").is_empty());
        assert!(completions("hi").is_empty());
    }
}
