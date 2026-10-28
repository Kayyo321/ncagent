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

pub const Commands: [command_t; 19] = [
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
        about: "turn animation on or off (saved in the settings)",
    },
    command_t {
        name: "/settings",
        args: "",
        about: "open the settings page",
    },
    command_t {
        name: "/links",
        args: "[osc8|off|inline|hidden]",
        about: "links as terminal hyperlinks or not; addresses shown or hidden (saved in the settings)",
    },
    command_t {
        name: "/theme",
        args: "[name|import <path>]",
        about: "list, apply or import a theme",
    },
    command_t {
        name: "/folder",
        args: "[name]",
        about: "new folder at the top level",
    },
    command_t {
        name: "/tab",
        args: "",
        about: "a new chat in a new tab",
    },
    command_t {
        name: "/close",
        args: "",
        about: "close this tab (a running turn stops first)",
    },
    command_t {
        name: "/rename",
        args: "[title]",
        about: "rename this conversation; no title opens the prompt",
    },
    command_t {
        name: "/pin",
        args: "",
        about: "pin or unpin this conversation",
    },
    command_t {
        name: "/archive",
        args: "",
        about: "archive or unarchive this conversation",
    },
    command_t {
        name: "/delete",
        args: "",
        about: "delete this conversation and its saved log",
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
    Settings,
    /// `/links osc8|off|inline|hidden`; with no argument, the report of both settings.
    Links(Option<crate::link_ui::LinksSwitch>),
    /// `/theme`: no argument opens the theme list, else the theme to apply (an id or a name).
    Theme(Option<String>),
    /// `/theme import <path>`: the path as typed, spaces included.
    ThemeImport(String),
    /// A new top-level folder: with a name it is created at once, without one the prompt opens.
    Folder(Option<String>),
    Tab,
    Close,
    /// A title for the open conversation; without one the prompt opens.
    Rename(Option<String>),
    Pin,
    Archive,
    Delete,
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
        "/settings" => simple(Command::Settings),
        "/links" => match (arg, extra) {
            (None, _) => Ok(Command::Links(None)),
            (Some("osc8"), None) => Ok(Command::Links(Some(crate::link_ui::LinksSwitch::Osc8))),
            (Some("off"), None) => Ok(Command::Links(Some(crate::link_ui::LinksSwitch::Off))),
            (Some("inline"), None) => Ok(Command::Links(Some(crate::link_ui::LinksSwitch::Inline))),
            (Some("hidden"), None) => Ok(Command::Links(Some(crate::link_ui::LinksSwitch::Hidden))),
            _ => Err("usage: /links [osc8|off|inline|hidden]".to_string()),
        },
        "/theme" => {
            let rest = text.get(name.len()..).unwrap_or("").trim();
            match rest.split_once(char::is_whitespace) {
                Some(("import", path)) => Ok(Command::ThemeImport(path.trim().to_string())),
                None if rest == "import" => Err("usage: /theme import <path>".to_string()),
                _ => Ok(Command::Theme((!rest.is_empty()).then(|| rest.to_string()))),
            }
        }
        "/quit" | "/exit" => simple(Command::Quit),
        "/tab" => simple(Command::Tab),
        "/close" => simple(Command::Close),
        "/pin" => simple(Command::Pin),
        "/archive" => simple(Command::Archive),
        "/delete" => simple(Command::Delete),
        "/folder" => {
            let rest = text.get(name.len()..).unwrap_or("").trim();
            Ok(Command::Folder(
                (!rest.is_empty()).then(|| rest.to_string()),
            ))
        }
        "/rename" => {
            let rest = text.get(name.len()..).unwrap_or("").trim();
            Ok(Command::Rename(
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
        assert_eq!(parse("/settings"), Some(Ok(Command::Settings)));
        assert_eq!(parse("/theme"), Some(Ok(Command::Theme(None))));
        assert_eq!(
            parse("/theme  Astral "),
            Some(Ok(Command::Theme(Some("Astral".into()))))
        );
        assert_eq!(
            parse("/theme import  ~/my themes/night.json "),
            Some(Ok(Command::ThemeImport("~/my themes/night.json".into())))
        );
        assert!(matches!(parse("/theme import"), Some(Err(_))));
        assert!(matches!(parse("/settings now"), Some(Err(_))));
        assert_eq!(parse("/quit"), Some(Ok(Command::Quit)));
        assert_eq!(parse("/folder"), Some(Ok(Command::Folder(None))));
        assert_eq!(
            parse("/folder  Research notes "),
            Some(Ok(Command::Folder(Some("Research notes".into()))))
        );
        assert_eq!(parse("/tab"), Some(Ok(Command::Tab)));
        assert_eq!(parse("/close"), Some(Ok(Command::Close)));
        assert_eq!(parse("/pin"), Some(Ok(Command::Pin)));
        assert_eq!(parse("/archive"), Some(Ok(Command::Archive)));
        assert_eq!(parse("/delete"), Some(Ok(Command::Delete)));
        assert_eq!(parse("/rename"), Some(Ok(Command::Rename(None))));
        assert_eq!(
            parse("/rename  The plan  "),
            Some(Ok(Command::Rename(Some("The plan".into()))))
        );
        assert!(matches!(parse("/pin now"), Some(Err(_))));
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
        assert_eq!(names, vec!["/cancel", "/clear-queue", "/copy", "/close"]);
        assert_eq!(completions("/").len(), Commands.len());
        assert!(completions("/help x").is_empty());
        assert!(completions("hi").is_empty());
    }
}
