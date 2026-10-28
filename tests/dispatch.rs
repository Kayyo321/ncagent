/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
//! A0: every command that is not done returns a named error, exits with status 1 and prints
//! nothing on stdout. These tests run the built binary.

use std::process::{Command, Output, Stdio};

fn ncagent(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ncagent"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the ncagent binary starts")
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_named_error(args: &[&str], needle: &str) {
    let output = ncagent(args);
    assert_eq!(output.status.code(), Some(1), "{args:?} must exit with 1");
    assert!(
        output.stdout.is_empty(),
        "{args:?} must print nothing on stdout"
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.starts_with("ncagent: error: "),
        "{args:?} must print a named error, got {stderr:?}"
    );
    assert!(
        stderr.contains(needle),
        "{args:?} error must mention {needle:?}, got {stderr:?}"
    );
}

#[test]
fn unfinished_commands_return_a_named_error_with_their_milestone() {
    assert_named_error(&["inspect", "/nonexistent"], "milestone A1");
    assert_named_error(&["exec", "/nonexistent", "--prompt", "hi"], "milestone H1");
    assert_named_error(&["serve", "/nonexistent"], "milestone U3");
    assert_named_error(
        &["generate", "/nonexistent", "--prompt", "hi", "--seed", "3"],
        "milestone N1",
    );
    assert_named_error(&["convert", "/a", "--out", "/b"], "milestone C1");
}

#[test]
fn chat_selects_the_real_backend_and_gets_the_not_implemented_error() {
    assert_named_error(&["chat", "/nonexistent"], "not implemented");
    let stderr = stderr_of(&ncagent(&["chat", "/nonexistent"]));
    assert!(
        stderr.contains("was not read or validated"),
        "chat must not pretend to read the export, got {stderr:?}"
    );
}

#[test]
fn demo_without_a_terminal_fails_with_a_named_error_before_touching_the_terminal() {
    assert_named_error(&["demo", "--ephemeral"], "needs a terminal");
}

#[test]
fn a_usage_error_exits_with_1() {
    let output = ncagent(&["no-such-command"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let output = ncagent(&["chat"]);
    assert_eq!(output.status.code(), Some(1), "a missing model directory");
}

#[test]
fn help_and_version_exit_with_0() {
    for args in [&["--help"][..], &["--version"][..], &["demo", "--help"][..]] {
        let output = ncagent(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        assert!(!output.stdout.is_empty(), "{args:?} prints to stdout");
    }
}
