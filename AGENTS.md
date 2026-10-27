# AGENTS.md

Instructions for AI coding agents working in this repository. Read this file and PROJECT.md before you change anything.

## Read first

1. [README.md](README.md): what ncagent is and how to build it.
2. [PROJECT.md](PROJECT.md): scope, pinned facts, architecture, decisions and milestones. It is the source of truth for what this project is, and it contains no open questions.
3. This file.

If a task conflicts with PROJECT.md, stop and ask. Do not change the scope on your own. Start at the first milestone in PROJECT.md that is not done, and do not start one before the milestones it depends on are done.

## Glossary

- **NCM**: Next-Concept Modeling (arXiv:2610.14162, HazyResearch). Never expand it as anything else.
- **Reference**: the HazyResearch/ncm commit recorded in PROJECT.md (`c70274273f489abc73832b2b98584a8483b6b203`).
- **Released checkpoint**: a folder in `hazyresearch/ncm-checkpoints`, as listed in PROJECT.md.
- **Export directory**: the four-file layout in PROJECT.md, "Export format". ncagent reads only this layout.
- **Converter**: `crates/convert`, which turns a released checkpoint into an export directory (D7).
- **Level**: one scale of the residual VQ hierarchy. Positions within a level are generated in parallel.
- **Pass**: one generator forward pass, one per level. Passes are the unit of cost.
- **Window**: the prefix length `P`, which bounds the prompt a model sees.
- **Orchestrator**: a model with the tool protocol in its window. **Worker**: a model given one bounded task with no tool list.
- **Session**: a transcript, its pinned state and its event log. **Side chat**: a session forked from another.

## Sources of truth

Before you state a fact (a Rust API, a crate's options, a file-format field, an NCM behaviour, a tensor name, a hyperparameter or a number), check it in the first source that applies:

1. Local code you can open: this repository, the locked crate sources (`cargo doc --open` or the vendored source under `~/.cargo/registry`), and the reference at the pinned commit.
2. Specifications and the paper: the safetensors specification, arXiv:2610.14162, the Hugging Face listing for the checkpoints, and docs.rs for a crate at the locked version.
3. Your memory. Use it only for general programming ideas, and check those against step 1 or 2.

Do not rely on memory for crate APIs, file byte layouts, tensor names or NCM hyperparameters. If you cannot verify something, research it until it is verified, and cite the source. If a choice must be made, make it, record the decision and its basis in PROJECT.md's decision list before the code that depends on it, and say so in your report. Do not leave a question open, and do not fill a gap with a plausible guess.

## Toolchain

- Rust 1.99.1 is the pin. `rust-toolchain.toml` sets it, and `rustc --version` must print `rustc 1.99.1` before any build. Install it with rustup if it does not.
- Edition 2024, set in the workspace `Cargo.toml`. The workspace sets `rust-version` to the same pin.
- Crate versions are pinned in `Cargo.lock`, which is committed. Each new crate needs a PROJECT.md entry and a licence check before it is added (D21).
- Lints: the workspace turns on clippy's default set and `-D warnings`. The only rustc lints allowed off are `non_camel_case_types` and `non_upper_case_globals`, and only at the crate roots, because of the owner's naming rules (PROJECT.md, "Language and toolchain").
- Formatting: `cargo fmt --all --check` lists files that do not conform. `cargo fmt --all` fixes them.

## Commands

Run these from the repository root.

- `cargo build`: build the workspace. Release builds use `cargo build --release`.
- `cargo test --workspace`: run all tests.
- `cargo clippy --workspace --all-targets -- -D warnings`: lint everything.
- `cargo fmt --all --check`: check formatting.
- `cargo run -- inspect <model-dir>`: check an export directory. Available from milestone A1.

## Source file header

- Every Rust, C and C++ source file begins with the owner's block form, exactly as written in PROJECT.md:

  ```
  /*
   * Ko, (Baczynski-Bruce) 2026; Apache 2.0
  */
  ```

- Python and shell files begin with `#` lines, as shown in PROJECT.md. A shebang line, when present, comes first.
- Data, configuration and Markdown files carry no header.
- The header must sit in the first three lines.

## Code rules

- The coding style below is the owner's rule. It applies to every language in the repository, and a reviewer checks it.
- One concern per change. Keep each diff small enough to review.
- Crate boundaries are fixed by PROJECT.md, "Architecture". `ncm` and `harness` never depend on `convert` or `tui`, and only the binary depends on `convert`.
- `ncm` and `harness` do no terminal I/O. Only `src/main.rs` and `crates/tui` know about the terminal.
- Model output, tool results, files on disk, and the released pickles are untrusted input. Parsers and readers return typed errors. They do not panic. Do not use `unwrap` or `expect` on any path that reads them. Tests cover malformed input.
- `unsafe` is allowed only in `crates/ncm` kernels and in the memory-map wrapper. Each `unsafe` block has a `// SAFETY:` comment that states the invariant it relies on.
- Every kernel has a scalar version. The scalar version is the test oracle for the SIMD version.
- The inert unpickler executes nothing. It never calls into the Python runtime, and it never treats a pickle global as a callable.
- The runtime never loads pickles. Only `crates/convert` reads them, and only under PROJECT.md decision D7.
- The generator is one code path configured by the export's flags. Do not add a per-family code path. Add a flag and a test instead.
- If an NCM feature is not implemented, return an error that names the feature. Do not approximate silently.
- Do not leave commented-out code, stub functions that return fake results, or placeholder numbers.
- Do not invent CLI flags, file formats, tensor names, config keys, tool names, benchmark results or speed claims. If something is not in the specification, the pinned code or a measured artifact, it does not exist yet.
- When you describe NCM behaviour, cite the pinned file and symbol, for example `models/next_concept.py` (`NextConceptTransformer`).
- The tool registry (D35) is the only list of tools. Do not declare a tool anywhere else.

## Coding style

- Never use camelCase, in any language in this repository.
- Use PascalCase only for constants, enums and preprocessor macros. For example: `MaxHeaderSize`, `Dtype`, `Dtype.F32`.
- Everything else uses snake_case: functions, variables, fields, parameters, modules and file names.
- Type definitions end in `_t`, for example `header_t`, `tensor_info_t` and `bf16_t`. In Rust this covers `struct` and `type` definitions. Enum types are not type definitions, so they keep PascalCase (`Dtype`).
- Constants are module-level values. Local variables, including `const` locals, use snake_case.
- Rust standard-library and crate names keep their own spelling when we call them, for example `std::mem::size_of` or `Terminal::draw`.

In Rust, the crate roots carry the two allowances that the owner's rules need, and nothing else:

```rust
#![allow(non_camel_case_types, non_upper_case_globals)]
```

## Data and weights

- Never commit weights, tokenizer files, pickles, datasets, caches, logs, generated export directories, or the downloaded `artifacts/` folder. Check `git status` before you report.
- `target/` is never committed.
- Tests write their synthetic export directories at test time. Commit only text fixtures: JSON, headers as JSON, and expected values as JSON.
- Golden tests use fixtures that the Python helper in `tools/` wrote from the pinned reference. The helper is a developer tool, and no test runs Python. The fixtures are committed as text.

## Git

- Do not commit, push, rewrite history or open pull requests unless the user asks.

## Verification before you report

Run these and include the results in your report:

1. `cargo test --workspace`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo fmt --all --check`
4. Header check. This must print nothing:

   ```bash
   for f in $(git ls-files --cached --others --exclude-standard '*.rs' '*.py' '*.sh'); do head -n 3 "$f" | grep -qF "Ko, (Baczynski-Bruce) 2026; Apache 2.0" || echo "missing header: $f"; done
   ```

5. Weight and pickle check. This must print nothing:

   ```bash
   git ls-files --cached --others --exclude-standard | grep -E '\.(safetensors|gguf|pt|pth|bin|pickle|pkl|onnx|ckpt)$'
   ```

6. Dependency-direction check. This must print nothing. It fails if `ncm` or `harness` depends on `convert` or `tui`:

   ```bash
   grep -nE "convert|tui" crates/ncm/Cargo.toml crates/harness/Cargo.toml
   ```

7. Runtime check. This must print nothing. The pickle reader is reachable only from `crates/convert`:

   ```bash
   grep -rniE "pickle|unpickle" crates/ncm crates/harness crates/tui src --include='*.rs'
   ```

8. Naming heuristic. This prints candidates, and a person reviews each one. Enum variants, constants and crate-level allowances are expected. A function, variable or module in PascalCase or camelCase fails. A type that does not end in `_t` is also a failure, unless it is an enum:

   ```bash
   grep -rnE "\b(fn|let( mut)?|mod|struct|type|static|const) +[A-Za-z_]*[A-Z]" crates src --include='*.rs'
   ```

Never say that tests pass unless you ran them in this task. Quote the command and the last lines of its output. If you could not run something, say what blocked it.

## Report format

End every task with: the changed files, the commands you ran with their results, anything you could not verify and why, and each decision you made with its basis.
