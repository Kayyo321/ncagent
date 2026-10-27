@AGENTS.md

# Claude Code notes

These notes apply only to Claude Code. The rules for every agent are in AGENTS.md, imported above.

- Read a file before you edit it. Use Grep or Glob to locate code instead of guessing paths.
- Before you call a crate API, read it at the locked version: `cargo doc --open -p <crate>` or the source under `~/.cargo/registry`. Do not rely on remembered signatures.
- Use your task list for multi-step work. Mark a step complete only after its verification has run.
- Record research notes and open decisions in PROJECT.md, not only in the chat, so the next session can read them.
- Check any released-checkpoint fact against the live Hugging Face listing before you rely on it. The reference README and the listing already disagree about OpenWebText (PROJECT.md, R12).
- Run `rustc --version` first. The pin is in `rust-toolchain.toml`, and a build with another toolchain does not count as verification.
