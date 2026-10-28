/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! Opening a link in the system browser (PROJECT.md, D76). This is the only code in the UI
//! that starts a process.
//!
//! The UI calls a `link_opener_t`. `system_opener_t` is the real one and is used only by
//! `tui::run`; tests use `recording_opener_t`, so no test starts a process. The real opener
//! checks the address again with the rule of D77, then starts the platform's URL opener with no
//! shell: the address is one argument of its own and never part of a command string. Its
//! standard streams are null, and it is not waited on; a short-lived thread reaps it so it does
//! not stay a zombie.

use std::fmt;
use std::sync::Mutex;

use crate::links::safe_url;

/// Why a link did not open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The address failed the check of D77.
    Refused { url: String },
    /// The opener could not be started: missing program, no desktop, no permission.
    Spawn { program: String, message: String },
    /// No URL opener is known for this operating system.
    Unsupported,
}

/// A failed open, with what went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct opener_error_t {
    pub kind: OpenError,
}

impl fmt::Display for opener_error_t {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            OpenError::Refused { url } => write!(f, "the address {url:?} is not an http(s) link"),
            OpenError::Spawn { program, message } => {
                write!(f, "could not start {program}: {message}")
            }
            OpenError::Unsupported => write!(f, "no URL opener is known for this system"),
        }
    }
}

impl std::error::Error for opener_error_t {}

/// Something that opens a checked address in the user's browser.
pub trait link_opener_t: Send + Sync {
    fn open(&self, url: &str) -> Result<(), opener_error_t>;
}

/// The program and its arguments that open `url` on `os` (a `std::env::consts::OS` value). The
/// address is always the last argument, whole. `None` for a system with no known opener.
pub fn opener_argv(os: &str, url: &str) -> Option<(String, Vec<String>)> {
    match os {
        "linux" | "freebsd" | "openbsd" | "netbsd" | "dragonfly" => {
            Some(("xdg-open".to_string(), vec![url.to_string()]))
        }
        "macos" => Some(("open".to_string(), vec![url.to_string()])),
        "windows" => Some((
            "rundll32".to_string(),
            vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()],
        )),
        _ => None,
    }
}

/// The real opener: the platform's URL opener, started as described in the module note.
#[derive(Clone, Copy, Debug, Default)]
pub struct system_opener_t;

impl link_opener_t for system_opener_t {
    fn open(&self, url: &str) -> Result<(), opener_error_t> {
        // Defence in depth: the address is checked again right before it becomes an argument.
        let Some(url) = safe_url(url) else {
            return Err(opener_error_t {
                kind: OpenError::Refused {
                    url: url.to_string(),
                },
            });
        };
        let Some((program, args)) = opener_argv(std::env::consts::OS, &url) else {
            return Err(opener_error_t {
                kind: OpenError::Unsupported,
            });
        };
        let child = std::process::Command::new(&program)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|err| opener_error_t {
                kind: OpenError::Spawn {
                    program: program.clone(),
                    message: err.to_string(),
                },
            })?;
        // Not waited on here: a thread reaps it whenever it ends.
        let mut child = child;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

/// A test opener: it records each address and starts nothing. `fail` makes every open fail.
#[derive(Debug, Default)]
pub struct recording_opener_t {
    pub opened: Mutex<Vec<String>>,
    pub fail: Mutex<Option<opener_error_t>>,
}

impl recording_opener_t {
    pub fn opened(&self) -> Vec<String> {
        self.opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn fail_with(&self, err: Option<opener_error_t>) {
        *self
            .fail
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = err;
    }
}

impl link_opener_t for recording_opener_t {
    fn open(&self, url: &str) -> Result<(), opener_error_t> {
        if safe_url(url).is_none() {
            return Err(opener_error_t {
                kind: OpenError::Refused {
                    url: url.to_string(),
                },
            });
        }
        if let Some(err) = self
            .fail
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Err(err);
        }
        self.opened
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(url.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hostile_address_stays_one_argument_on_every_platform() {
        let hostile = "https://e.example/a b&c;d|e'f\"g$(rm -rf ~)`x`>y<z";
        for (os, program, before) in [
            ("linux", "xdg-open", 0usize),
            ("macos", "open", 0),
            ("windows", "rundll32", 1),
        ] {
            let Some((p, args)) = opener_argv(os, hostile) else {
                panic!("{os} has an opener");
            };
            assert_eq!(p, program);
            assert_eq!(args.len(), before + 1, "{os}: {args:?}");
            assert_eq!(args.last().map(String::as_str), Some(hostile), "{os}");
        }
        let Some((_, win)) = opener_argv("windows", "https://e.example") else {
            panic!("windows");
        };
        assert_eq!(
            win.first().map(String::as_str),
            Some("url.dll,FileProtocolHandler")
        );
        assert_eq!(opener_argv("plan9", "https://e.example"), None);
    }

    #[test]
    fn the_real_opener_refuses_a_bad_address_before_starting_anything() {
        for bad in [
            "javascript:alert(1)",
            "https://e.example/a b",
            "file:///etc/passwd",
        ] {
            assert!(matches!(
                system_opener_t.open(bad).map_err(|e| e.kind),
                Err(OpenError::Refused { .. })
            ));
        }
    }

    #[test]
    fn the_recording_opener_records_and_can_fail() {
        let o = recording_opener_t::default();
        assert!(o.open("https://e.example/x").is_ok());
        assert!(o.open("ftp://e.example").is_err());
        o.fail_with(Some(opener_error_t {
            kind: OpenError::Spawn {
                program: "xdg-open".into(),
                message: "No such file or directory".into(),
            },
        }));
        assert!(o.open("https://e.example/y").is_err());
        assert_eq!(o.opened(), ["https://e.example/x"]);
    }
}
