# ncagent

ncagent is a command-line agent for Next-Concept Modeling (NCM) models. It runs an NCM model from a local directory and gives it the features of a production coding agent: sessions that resume, tool calls with approvals, compaction, sub-agents, side chats, a terminal interface with mouse support, and a headless mode for editors and apps. It pairs with [ncmorph](https://github.com/Kayyo321/ncmorph), which converts open-weight language models into NCM models.

**Status: the agent is not implemented. A UI prototype runs against a simulator.** No NCM model runs in this repository yet. The workspace builds, and `ncagent demo` starts the terminal UI against a scripted, clearly labelled demo backend: its replies and tools are simulated, no model runs and nothing is executed (owner exception O1 in [PROJECT.md](PROJECT.md)). `ncagent chat <model-dir>` returns a named not-implemented error until the engine exists.

## Try the UI prototype

Build with the pinned toolchain (Rust 1.99.1, set by `rust-toolchain.toml`), then run the demo in a terminal:

```bash
cargo run -- demo
```

Type a trigger word to see a scripted scenario: `tool`, `approve`, `deny`, `shell`, `ask`, `fail`, `error`, `flaky`, `long` or `markdown`. Press `?` for the keys. The options are `--profile standard|compact`, `--speed normal|fast|instant`, `--approval ask|edit|full`, `--seed`, `--state-dir`, `--ephemeral` and `--no-mouse`. Demo sessions are saved as JSON lines in your local data directory, marked as simulated (PROJECT.md, D40). Demo folders are saved in `folders.json` next to the demo sessions (PROJECT.md, D50), and `--ephemeral` keeps them in memory. To preview the UI without any backend, run `cargo run -p tui --features preview --example gallery`.

## What it will do

- Run an NCM model on the CPU, with no GPU and no network.
- Run an agent in your terminal, with a mouse-aware interface. Clicking and scrolling work, and every mouse action has a keyboard path.
- Ask before it writes files or runs commands. Approval modes are in PROJECT.md (D15).
- Keep sessions in a local log, resume them, fork side chats, and compact long histories.
- Start sub-agents with pass budgets, each in its own git worktree when it writes.
- Operate other terminal programs: open them, read their screens as text, click, scroll and type (D29).
- Run headless with `exec`, or drive sessions from an editor with `serve`.

## Read this first

NCM is not an ordinary language model, and the agent is designed around that.

- **A reply is one block.** The model writes all of a reply at once, refining it from coarse to fine. You see progress per level, not a stream of words. A stop request takes effect after the current level.
- **The window is small.** A model sees only P tokens of prompt, between 91 and 512 for the released conditional models. Compaction and sub-agents are therefore core features, not extras.
- **Replies are short.** Each call returns at most L tokens: 48 for XSum, 64 for WMT14, 89 for Sudoku, 128 for LM1B and TinyStories. Large edits are made in small steps.
- **The released models are not agents.** They were trained for unconditional text, summarization, translation and Sudoku. None follows instructions or calls tools. The released XSum model can serve as a summary worker for compaction. A model trained for the agent protocol is needed for real agent work (PROJECT.md, R1).

## Requirements

- To build: the Rust toolchain, installed with rustup. The project pins Rust 1.99.1 in `rust-toolchain.toml`.
- To run a built binary: nothing else. It needs no Python, no GPU and no network.
- To download the released checkpoints: the Hugging Face `hf` command.

## Released models

The Hugging Face repository [hazyresearch/ncm-checkpoints](https://huggingface.co/hazyresearch/ncm-checkpoints) holds these checkpoints. Each model folder has a `config.yaml`.

| Checkpoint | Mode | Window P | Reply length L |
|---|---|---|---|
| `lm1b` small, medium, large | unconditional | 1 | 128 |
| `tinystories` | unconditional | 1 | 128 |
| `xsum` | article to summary | 512 | 48 |
| `wmt14` | German to English | 96 | 64 |
| `sudoku` easy, medium, hard | puzzle to solution | 91 | 89 |

The reference README also lists OpenWebText. The repository holds only `owt/TODO.md`, so no OpenWebText model is available.

Download them with the reference's command. It fetches about 8 GB:

```bash
hf download hazyresearch/ncm-checkpoints --local-dir artifacts
```

The checkpoints are PyTorch pickles, so the binary cannot read them directly. `ncagent convert` turns each folder into an export directory once. The binary then reads only export directories.

## Planned commands

`build`, `test` and `demo` work today. `inspect` and `chat` are planned, and return a named error until their milestones are done.

```bash
cargo build --release
```

```bash
cargo test --workspace
```

```bash
cargo run -- inspect <model-dir>
```

```bash
cargo run -- chat <model-dir>
```

## Contributing

Read [AGENTS.md](AGENTS.md) before you change anything. Every source file starts with the copyright line defined in [PROJECT.md](PROJECT.md), in that language's comment syntax.

## Credits

ncagent builds on **Next-Concept Modeling**, developed by Hermann Kumbong, Ishan S. Khare, Iddah Mlauzi, Andrew Woen, Charles Ding, E. Kelly Buchanan, Simran Arora, Azalia Mirhoseini and Christopher Ré in the HazyResearch group at Stanford.

- Paper: [arXiv:2610.14162](https://arxiv.org/abs/2610.14162)
- Reference code and checkpoints: [github.com/HazyResearch/ncm](https://github.com/HazyResearch/ncm) (MIT License) and the [NCM collection on Hugging Face](https://huggingface.co/collections/hazyresearch/ncm)

Thank you to the NCM authors. Their research, and their decision to release the paper, code and weights openly, made ncagent possible.

## License

ncagent is licensed under the Apache License 2.0 (see [LICENSE](LICENSE)). The NCM reference is MIT-licensed, and each export's `NOTICE` file keeps its notice.
