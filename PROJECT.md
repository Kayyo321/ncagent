# ncagent

ncagent is a production-grade agent harness for Next-Concept Modeling (NCM) models, shipped as one command-line program. It runs an NCM model from a local directory and gives the model what Codex and Claude Code give theirs: sessions that survive restarts, tool calls with approvals, compaction, sub-agents, side chats, a full-screen terminal interface with mouse support, and a headless mode for editors and apps. [ncmorph](https://github.com/Kayyo321/ncmorph) converts open-weight language models into NCM models. ncagent runs them.

NCM is not an autoregressive language model. Most of the harness design therefore differs from Codex and Claude Code. The section "What NCM changes for an agent" sets out those differences, and each later section follows from it.

This file is the specification. It states the scope, the pinned sources, the facts the implementation depends on, every decision with its basis, and the milestones that define done. It contains no open questions. [AGENTS.md](AGENTS.md) holds the working rules, and [README.md](README.md) is the user guide.

## Status

Specification only. The repository holds the licence and the four documents. No code exists. The first milestone is A0.

This revision changes the language to Rust (D1) and replaces the earlier harness outline with the NCM-specific design below. It supersedes the Zig and Python decisions of the previous revision, which are listed under "Superseded decisions".

## Source file header

The owner asked for every source file to begin with this copyright line, in the owner's block form:

```
/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
```

Each language keeps that text in its own comment syntax (D22).

- **Rust, C and C++**: the block form, exactly as written. Rust has block comments, so no change is needed.
- **Python and shell**: `#` lines. A shell script's shebang line, when present, comes first.

  ```python
  #
  # Ko, (Baczynski-Bruce) 2026; Apache 2.0
  #
  ```

- **Markdown, TOML, JSON, YAML, fixtures and other data or configuration files**: no header.

## Language and toolchain

ncagent is written in Rust (D1). It is one language for the command, the terminal interface, the harness and the NCM engine. Python is not needed to install or run ncagent. A developer may use Python in `tools/` to generate test fixtures (D2).

| Option | Verdict | Reason |
|---|---|---|
| Rust 1.99.1 | Chosen | The terminal interface library (ratatui on crossterm 0.29.0) reports mouse press, release, drag and all four scroll directions, and it runs on the Windows console. The official Model Context Protocol SDK (`rmcp`) is in Rust. Async tasks (tokio) carry sub-agents and side chats. Compiled code runs at native speed, and SIMD is available through `std::arch`. One binary needs no interpreter or runtime. Codex is written in Rust (github.com/openai/codex), and its maintainers moved it from Node.js to Rust for speed and to avoid the Node dependency (DevClass, June 2025). |
| Zig 0.17.0 | Rejected for ncagent | Used by ncmorph and matches the owner's naming rules. For this scope it lacks the parts. The one Zig terminal library with mouse support, libvaxis, is small, and its sources disagree on the Zig version it needs (0.14, 0.15.1, 0.16.0). Zig has no official MCP SDK. Its async I/O API (`std.Io`) changed between 0.16 and 0.17, and ncmorph's notes cover 0.16 only. |
| Go | Rejected | Good concurrency and a single binary. The NCM kernel needs SIMD, and Go has no SIMD intrinsics, so the hot loops would need assembly. Go also makes exported names capitalised, which conflicts with the owner's snake_case rule for functions. |
| TypeScript on Node.js | Rejected | Requires a Node runtime on every machine. Codex left that model for this reason. The secondary sources for Claude Code describe the same prerequisite (Node.js 18 or newer); they are not confirmed against Anthropic's own documentation. |
| Python | Developer test fixtures only | The released checkpoints are PyTorch pickles, but the converter does not need Python (D7). Python is kept only for the reference outputs that tests compare against (D2). |

Rust naming and the owner's rules. The owner's style rules (AGENTS.md, "Coding style") ask for `_t` on every type name and PascalCase for constants. The Rust compiler warns on both by default, so the crate roots allow exactly two lints: `non_camel_case_types` and `non_upper_case_globals`. Functions, variables, modules and files stay snake_case, which is already Rust's convention. Enum variants stay PascalCase, which is also Rust's convention. No other lint is allowed off.

Prerequisites:

- To build from source: the Rust toolchain through rustup, pinned by `rust-toolchain.toml` to 1.99.1. The stable release was read from releases.rs on 2026-10-26, and it lists 1.99.1 as current. The machine checked on 2026-10-26 has rustc 1.98.0, so the pin must be installed before any build.
- To run a released binary: nothing.
- To read a released checkpoint: nothing. The converter is part of the binary (D7).
- To download checkpoints: the Hugging Face `hf` command, which is the user's responsibility (D5).

## What NCM changes for an agent

These are the facts that drive the harness design. Each fact has a source, and the consequence follows it.

1. **A reply is one block, not a stream.** A call produces L text positions at once. The positions are refined over K level passes, coarse to fine. Text appears when the finest level is done. The interface can show progress per level, and a stop request takes effect after the current level pass. Mid-reply steering does not exist.

2. **Positions do not see each other inside a level.** Within a level, every position is sampled in parallel. Coherence comes from the coarser levels only. So a grammar that forces valid syntax token by token is not available. Syntax is a property of the finished reply, and the harness checks it after the call and recovers by sampling again (D16).

3. **Every reply has the same length.** A call yields exactly L text positions. The reference's decoder emits one logit row per position, and the harness cuts the text at the first EOS when `cut_at_eos` is set, which is the reference default (`scripts/eval_shard.py` lines 130 to 134). A reply that uses all L positions without an EOS is marked truncated, and the rest comes in a new call (D11). Large content cannot fit one reply, so file changes are made in small operations (see "Reply protocol").

4. **The prompt is a fixed window.** The model sees only its prefix of P tokens. P is a training-time setting of each model. The released models have P of 1, 91, 96 and 512 (see "Released NCM models"). The harness must fit the protocol, the tool list, the pinned state, a summary and recent turns into P (D10).

5. **The prefix is bidirectional and independent of the target.** Prefix positions attend to each other and never to the target levels (`models/next_concept.py`, the block mask at lines 266 to 292; ncmorph research, `ncm_facts.md` A2). So the prefix's hidden states are the same at every level of one call. The harness computes them once per call and reuses them for all K levels. They cannot be reused across calls, because a changed prompt changes the bidirectional prefix. This is a derivation from the mask. It is checked against the reference in N1.

6. **Cost is counted in generator passes.** A call runs one generator pass per level. The count is K, or K minus one when the level-0 codes come from the stored unigram (`ncm_facts.md` A4). Classifier-free guidance doubles the passes (`ncm_facts.md` E2). A user turn therefore has a budget in passes, and the interface shows the spend.

7. **Independent calls are cheap to run side by side.** Two calls share nothing except the loaded weights, which are read-only. Sub-agents and side chats are sets of calls that run in parallel, bounded by CPU and memory.

8. **Truncated levels can be decoded.** The tokenizer is trained with nested dropout. The decoder receives reconstructions built from a random number of the first levels, between `min_keep` and K minus one (`models/quant.py`, `fine_drop`; paper Eq. 9). So a reconstruction from the first levels is an input the decoder was trained on. The harness can decode a draft after any level, or stop early. Draft quality is not measured yet, and N4 decides whether drafts are shown.

9. **Candidates can be compared.** The training objective (paper Eq. 10) gives a likelihood-like score for a complete set of codes. That allows best-of-N with a selection rule. The paper's MBR (minimum Bayes risk) picks the candidate most similar to the others (paper Section F.4). The score is a bound, not an exact likelihood, and its value for ranking is unmeasured.

10. **The model cannot see the screen.** NCM is a text model. ncmorph's scope excludes vision. A model that operates a terminal or a desktop can only read text. The harness therefore gives the agent a character grid with row and column numbers, and never pixels (D29).

11. **The released models do not follow instructions.** Their training data is unconditional text, summarization, translation and Sudoku (paper Table 4). None of it is dialogue or tool use. The harness runs these models for mechanics only, until a model is trained for the protocol. The released XSum model is trained to turn an article into a summary of at most 48 tokens, which is the job compaction needs (see "Compaction").

## Scope

In scope:

- Sessions with a persistent event log, resume, and fork (D24).
- Packing a session into a model's window (D10).
- The reply protocol with tool calls, questions to the user, and a retry policy (D13, D16).
- Tools, approvals and a shell (D15, D17).
- Compaction in four layers (D25).
- Sub-agents with budgets and worktree isolation (D26).
- Side chats (D27).
- A terminal interface with mouse and keyboard, and a terminal driver that lets the agent operate other terminal programs (D28, D29).
- A headless mode for `exec` and for `serve` over JSON-RPC (D30).
- Project instruction files (D31).
- Conversion of released checkpoints inside the binary (D7).

Later, and each with its own research step before design:

- Model Context Protocol servers through `rmcp` (D32, milestone M1).
- Sandboxing of shell commands on each operating system (S1).
- Pointer control of the desktop outside a terminal (X1). It needs text-only perception and is off by default.

Out of scope:

- Training, fine-tuning and distillation. The reference has no training code (`ncm_facts.md` C5).
- GPU execution.
- Network access inside the harness. Downloads stay with the user (D5).
- Quantized weights, GGUF files and pickle files outside the converter (D7).
- Any speed or quality claim that has not been measured (D20).

## Released NCM models

The source is the Hugging Face repository `hazyresearch/ncm-checkpoints`, licence tag `mit`, at revision `d342ce8aaea58830eef3b8ad473664673e41f692`. That is its latest commit as read on 2026-10-26. The repository holds 12 folders and 35 files. Sizes are in bytes, from the listing.

| Checkpoint | Files | Generator / tokenizer bytes | Text tokenizer | Prefix P | Length L | Mode | Usable as |
|---|---|---|---|---|---|---|---|
| lm1b small | `lm1b/small/generator.pt`, `config.yaml`; shared `lm1b/tokenizer.pt` | 390,138,443 / 597,840,331 (shared) | bert-base-uncased | 1 | 128 | unconditional | none for agents |
| lm1b medium | `lm1b/medium/generator.pt`, `config.yaml` | 1,268,014,283 | bert-base-uncased | 1 | 128 | unconditional | none for agents |
| lm1b large | `lm1b/large/generator.pt`, `config.yaml` | 2,901,184,267 | bert-base-uncased | 1 | 128 | unconditional | none for agents |
| tinystories | `generator.pt`, `tokenizer.pt`, `bpe_merges_8192.pickle`, `bpe_vocab_8192.pickle`, `config.yaml` | 356,160,651 / 221,861,835 | custom BPE, 8,192 | 1 | 128 | unconditional | none for agents |
| xsum | `generator.pt`, `tokenizer.pt`, `config.yaml` | 354,355,211 / 299,626,079 | GPT-2 | 512 (article) | 48 | article to summary | summary worker (compaction) |
| wmt14 | `generator.pt`, `tokenizer.pt`, `config.yaml` | 480,582,539 / 645,790,351 | Qwen3-0.6B | 96 (German) | 64 | German to English | translation worker |
| sudoku easy, medium, hard | `generator.pt`, `tokenizer.pt`, `config.yaml` in each | 102,560,203 / 59,816,209 in each | 12 symbols | 91 (puzzle) | 89 | puzzle to solution | none |
| owt | `owt/TODO.md` only | none | none | none | none | not released | none |

- The WMT14 pair matches the sizes and SHA-256 values that ncmorph read from the same repository (`tokenizer.pt` `74f04176a3cd985e4142761bd7464ea6edb44dc0025c40302b5e45ea0d865702`, `generator.pt` `ae580cbbb400a27b30c616634745d4c9cfa8b66806f936c6c26041e5b9640d4f`). The other SHA-256 values are recorded in T0 from a raw listing. They are not copied here.
- The reference README lists OpenWebText checkpoints. The listing holds only `owt/TODO.md`, so ncagent follows the listing (R12).
- The prefix P, length L and text tokenizer come from the reference configurations and from ncmorph's research (`ncm_facts.md` section F and Table 6). The converter reads them from each `config.yaml`, and `inspect` prints them.
- No released model is an agent. The "Usable as" column is the role each model can fill: a worker whose whole job is one bounded transformation, with no tool list in its window.

## Export format

ncagent reads the NCM export directory that ncmorph defines (ncmorph PROJECT.md, "Export format", decision D26). The released checkpoints are converted into the same layout (D7), so one reader serves both. An export directory holds four files.

| File | Contents |
|---|---|
| `ncm.safetensors` | Generator tensors: the input projection, the level embedding, the level-0 queries or read-out, the blocks, the final norm, the output heads, and the post-upsample convolutions, under the names the converter derives from the pinned reference. |
| `tokenizer.safetensors` | VQ tensors: the prefix token table, the decoder, the codebooks, and the level-0 resampling and residual-mixing tensors. |
| `ncm.json` | Every field the reader needs, listed below. Unknown fields are an error. |
| `NOTICE` | The licence and attribution text for each part (D21). |

The safetensors `__metadata__` map holds `ncmorph_format` with the value `1`. The key is ncmorph's name. The converter writes it too, so one reader handles both sources (D3).

`ncm.json` holds: the family; the source repository and revision; `K`, the level sizes, `L`, the codebook sizes and `d`; the prefix length `P`; every generator flag; the head type; the level-0 source; the pick rule and its settings; the temperature, `top_k` and `top_p`; `cut_at_eos`; the text tokenizer's id and revision; the tokenizer route; the chat template name (default `plain`, D14); and the approval-independent defaults a model needs, such as its guidance scale, which is absent unless the config sets it.

Before any weight is read, the reader derives the expected tensor list from `ncm.json` and checks the headers against it (ncmorph PROJECT.md, "Manifest validation"):

- Every expected tensor is present, with the expected shape and a supported dtype.
- No tensor is unexpected.
- Each header is at most 100,000,000 bytes. Offsets are contiguous. Each byte count equals shape × bit size / 8. All offset and size arithmetic is checked, and an overflow is an error.
- Supported storage dtypes are F32, BF16 and F16. Each tensor is widened to F32 at load (D6).

## NCM facts the runtime depends on

The reference is `HazyResearch/ncm` at commit `c70274273f489abc73832b2b98584a8483b6b203` (MIT). The paper is arXiv:2610.14162. Each fact is cited to the pinned commit. ncmorph's `research/ncm_facts.md` holds the full fact sheet with line numbers.

1. **Coarse to fine.** Level 0, with the fewest codes, is generated first. Each later level takes one forward pass, and all its codes are sampled in parallel. Before each pass, the reconstruction so far is resized to the level's length, projected and fed in (`models/next_concept.py`, `generate`, lines 521 to 703).
2. **Attention.** The mask is block-diagonal. The prefix attends to the prefix. Each level attends to the prefix and to itself, and never to another level (lines 266 to 292). Pad prefix tokens are masked as keys (lines 418 to 425 and 572 to 579).
3. **Positions.** The prefix takes positions `0` to `P-1`. Each level restarts at `0` to `ℓ_j-1`. RoPE is the positional mechanism. Where a config sets QK-norm, it runs before RoPE (lines 94 to 117; `models/common.py` lines 76 to 110).
4. **Prefix ids.** Prefix ids pass through the VQ encoder's token table and then the generator's input projection (lines 546 to 558). The unconditional prefix is one EOS id. The guidance null prefix is `[EOS, PAD, …]` (lines 534 to 535). Conditional prompts are right-padded to `P` (`scripts/eval_shard.py` lines 94 to 96).
5. **Level-0 source depends on the config.** XSum and Sudoku use learned queries, `l0_queries`. WMT14 reads level 0 from the prefix (`predict_level0`, lines 249 to 264). LM1B and TinyStories draw level-0 codes from the stored unigram `inference.level0_unigram` in `generator.pt` (`scripts/eval_shard.py` line 156).
6. **Heads.** XSum and TinyStories use tied codebook heads, with a learned scale and a per-code bias (`_TiedCodebookHead`, lines 9 to 34). LM1B uses one shared head. WMT14 and Sudoku use untied linear heads.
7. **Pick.** The token pick is argmax, unless the config sets a frequency or adjacency penalty. XSum and LM1B set one (`scripts/eval_shard.py` lines 99 to 124).
8. **Sampling.** XSum and WMT14 use temperature 1.0, `top_k` 1 and `top_p` 0.0, which is greedy. TinyStories and LM1B use temperature 1.0, `top_k` 0 and `top_p` 1.0. Sudoku uses temperature 0, which is argmax (`_sample_level_raw`, lines 487 to 499).
9. **Guidance.** The logits at each level are `(1 + t)·cond − t·uncond`, with `t = scale·j/(K−1)` (lines 477 to 485 and 528 to 540). The README's guided WMT14 row uses `--cfg 1.0`.
10. **Post-upsample convolution.** A zero-initialised `Conv1d` with kernel 3 runs on each upsampled input for levels `j ≥ 1`. It is present only in the OWT and LM1B configs (`next_concept.py` lines 183 to 195 and 667 to 668).
11. **Tokenizer.** The VQ modules are `models/vqvae.py`, `models/vae.py` and `models/quant.py`. The quantizer uses area down-sampling and linear up-sampling, a residual mix with `r = 0.5`, and a learned level-0 resampler (`ncm_facts.md` section C).
12. **Nested dropout.** Decoder inputs come from a random number of the first levels (fact 8 in "What NCM changes"). The reference's `fine_drop` keeps at least `min_keep` levels and never all K (`models/quant.py`).

The reference README gives no sampling command and no Python API. Its only entry point is `evaluate.py`, which generates as part of scoring. ncagent therefore takes generation behaviour from `models/next_concept.py` and `scripts/eval_shard.py`.

Where the paper and the shipped configs differ (XSum uses 9 levels in the config and 8 in the paper, the OWT codebook size differs, and the pick differs), ncagent follows the config. The released numbers were produced with the config (`ncm_facts.md` section H).

## Architecture

A Cargo workspace, with one binary and four library crates:

```
Cargo.toml                 workspace, shared lints
rust-toolchain.toml        pins Rust 1.99.1
src/main.rs                the ncagent binary: command dispatch only
crates/ncm/                export reader, kernels, tokenizers, generator, pick and sampling
crates/harness/            sessions, event log, packing, reply protocol, tools, approvals,
                           compaction, sub-agents, side chats, instructions
crates/tui/                ratatui front end: layout, mouse, approval and question widgets
crates/convert/            released-checkpoint converter and the inert unpickler (D7)
tools/                     Python developer helper that writes golden fixtures (D2)
```

Dependency rules, enforced by Cargo.toml (checked in AGENTS.md):

- `ncm` depends on nothing in the workspace.
- `harness` depends on `ncm`. It never depends on `convert` or `tui`.
- `tui` depends on `harness`. It never talks to `ncm` directly.
- `convert` depends on `ncm` for the export types. Nothing depends on `convert` except the binary.
- Only `src/main.rs` and `crates/tui` know about the terminal. `harness` and `ncm` are free of terminal I/O, so `exec` and `serve` run them without a screen.

Crates chosen in this specification, each to pass a licence check and to be pinned in `Cargo.lock` at the milestone that adds it (AGENTS.md, code rules):

| Crate | Use | Milestone |
|---|---|---|
| `safetensors` (Hugging Face) | reading the export tensors | A1 (D4) |
| `tokenizers` (Hugging Face) | text tokenizers, if it reproduces the reference's ids | T1 |
| `ratatui` with `crossterm` 0.29.0 | terminal layout and input | U1 |
| `tokio` | async session tasks, sub-agents, `serve` | H1 |
| `rayon` | parallel kernels and parallel calls | N1 |
| `serde` and `serde_json` | event log, JSON-RPC, tool results | H1 |
| `toml` | configuration | H1 |
| `clap` | command line | A0 |
| a pseudo-terminal crate with ConPTY support on Windows, and a VT parser | terminal driver | U2 |
| `rmcp` (official Model Context Protocol SDK, spec 2026-07-28) | MCP client | M1 |

The inert unpickler (D7) and the kernels (`std::arch`, with a scalar version of every kernel) are written in this repository.

Threads and state. Model weights load once, as read-only memory maps, and are shared through `Arc`. A session is a tokio task that owns its transcript and its state. Model calls run on a compute pool. The pool gives foreground work priority over sub-agents and side chats (D33). Cancellation is a token that the engine checks between level passes, and the tool runner checks between operations.

## The model interface

One engine call takes a prefix (token ids of length at most P), the export's sampler settings and a seed. It returns, through a callback after each level, the level index, the codes so far and an optional draft. It returns the final `L` token ids. The callback can cancel the call. Nothing else in the harness touches the engine.

Calls are recorded with their seed, prompt hash, and export identity (D24), so any call can be replayed.

## Prompt packing

The prefix is built from sections in a fixed order. The order is the same in every call, which makes the prompt easy to read and makes a failure in the window easy to explain.

| Order | Section | Contents | Budget (design choice, a fraction of P) |
|---|---|---|---|
| 1 | Protocol | The reply grammar and the tool registry, from one template | at most P/2, measured at A-step H1 |
| 2 | Pinned state | Task, working directory, approval mode, project instructions excerpt, the current plan | at most P/8 |
| 3 | Rolling summary | Output of compaction (D25) | at most P/4 |
| 4 | Recent turns | The newest turns verbatim, each cut to a per-turn limit, newest last | the rest |
| 5 | Pending results | The results of the last tool calls, each cut to 2,048 bytes | counted inside 4 |

Packing rules:

- Token counts come from the model's own tokenizer, never from character counts.
- If section 1 does not fit in P/2, tools are off for this model, and the model can only be a worker (D12).
- If sections 1 and 2 together do not fit in P, the model cannot be an orchestrator. The error names the model and its P.
- Right-padding goes after the last token, as the reference does (fact 4).
- The packed prompt and its hash go into the call's event record.

## Reply protocol

A reply is text. The harness reads it as a sequence of items.

- **Text**: any characters outside a block. The text is the assistant's message.
- **Call block**: `@call` followed by a tool name on the same line. Then one `key: value` line per argument, then `@end`.
- **Question block**: `@ask`, then `question:` and `options:`, then `@end`. Options are separated by `|`.

A call block looks like this:

```
@call read_file
path: src/main.rs
lines: 1 80
@end
```

Grammar rules:

- Tool names are exact and lowercase. No fuzzy matching in v1. A bad name is an invalid reply, and it is retried under D16. Fuzzy matching can be added after N-best data exists.
- Argument names are exact and snake_case. Each argument is one line. A newline inside a value is written `\n`, and a backslash is written `\\`.
- Missing, unknown or extra arguments make a call invalid.
- A field longer than its limit makes the call invalid. Limits are declared with each tool.
- Several call blocks in one reply run in the order written. Read-only calls may run in parallel with each other.
- Text after the last block is ignored when a call block is present, so the model cannot both act and claim a result in one reply. The claim is checked against the result.

Edits fit in small replies. A file change is a sequence of operations, each bounded to one reply's arguments (`replace_lines`, `insert_lines`, `delete_lines`, `create_file`, `append_lines`). A long file is written by many calls, and each call is shown for approval. A diff view shows the operations applied so far.

Reliability ladder (D16). When the harness needs a call or a question from the model:

1. Sample one reply. If it is valid, use it.
2. If not, sample again with a new seed, up to R attempts, with R set by the session's pass budget.
3. If N-best is enabled, draw N candidates for the same prompt in one batch. Keep the valid ones, and choose by consensus: the candidate whose calls match most of the others. This is the MBR rule (fact 9).
4. If nothing valid results, ask the user, and do not loop. The message names the failure.

Every attempt counts toward the turn's pass budget, and the interface shows it.

Truncation. A reply with no EOS in its L positions is marked `truncated`. The harness continues it with a new call whose prefix holds the partial reply, and the new reply is appended. The continuation is recorded as one turn.

## Tools

Tools are declared in one registry. Each tool has a name, typed arguments with length limits, an approval class, and a result limit.

| Tool | Arguments | Approval class | Notes |
|---|---|---|---|
| `list_dir` | `path` | read | Entries of one directory. |
| `read_file` | `path`, `lines` | read | Lines `a b`, or the whole file. Result cut to 2,048 bytes. |
| `search` | `pattern`, `path` | read | Fixed string in v1. Regex is added after a decision. |
| `replace_lines` | `path`, `start`, `end`, `text` | write | One change; shows a diff. |
| `insert_lines` | `path`, `after`, `text` | write | |
| `delete_lines` | `path`, `start`, `end` | write | |
| `create_file` | `path` | write | Creates an empty file. Fails if the file exists. |
| `append_lines` | `path`, `text` | write | For building a file in several calls. |
| `run_shell` | `command` | shell | Runs `sh -c` on POSIX and `cmd /c` on Windows. Output cut to 2,048 bytes; full output goes to a file. Stops after 60 seconds. |
| `spawn_agent` | `role`, `task`, `budget_passes`, `workspace` | agent | Sub-agents, D26. |
| `await_agent` | `id` | read | Waits for a sub-agent's result. |
| `cancel_agent` | `id` | read | |
| `search_history` | `query` | read | Searches the event log for older turns (D25). |
| `term_open`, `term_read`, `term_click`, `term_scroll`, `term_key`, `term_close` | see U2 | shell | Terminal driver, D29. |

Approval classes and modes (D15):

- `ask` (default): read tools run. Write, shell and agent tools ask. Each answer can be once, for this call only, or for the rest of the session, for this tool with the same path or command prefix.
- `edit`: write tools inside the working directory run without asking. Shell asks.
- `full`: every tool runs inside the working directory without asking. Any path outside the working directory asks. `full` must be selected explicitly on the command line, and the interface states it on every screen.

A declined call returns a tool error to the model and does not end the session. Under `exec`, there is no one to ask, so every call that would ask is refused. This is the fail-closed rule (D30).

Shell processes start in their own process group. Cancelling a call kills the group.

## Terminal interface

The interface is one full-screen program, built with ratatui over crossterm 0.29.0 (D28). Crossterm reports press, release, drag and move events, and the wheel as `ScrollUp`, `ScrollDown`, `ScrollLeft` and `ScrollRight`. U1 verifies the interface on Linux and macOS terminals and in the Windows console host. Crossterm's list of tested terminals includes the Windows Console Host. Its issue tracker reports partial mouse support under MSYS, Cygwin and mintty (crossterm issue 580), so those are not supported for mouse.

Layout and mouse behaviour:

- **Transcript**: scrolls with the wheel and with the scrollbar. Click a tool result to expand or fold it. Drag to select text, and copy with a keystroke.
- **Level strip**: K cells, one per level pass, filled as passes finish. Each cell shows the pass time. Clicking the strip cancels the call after the current pass. If drafts are enabled (N4), clicking a filled cell shows the draft decoded after that level.
- **Input line**: typing while a call runs queues a message. The queue is applied before the next call (see Steering).
- **Approval modal**: shows the exact command, or the diff of each write. Three buttons, each also on a key: once (`y`), session (`a`), deny (`n`).
- **Question block**: shows the options as buttons with numbers. A click or a number key selects one. A free-text answer is also allowed.
- **Tabs**: one per side chat, with the main session first. A click switches tabs.
- **Status line**: the model, the approval mode, the pass count of the turn, and the window use as tokens over P.

Keyboard parity. Every mouse action has a keyboard path. The keyboard path is tested. Terminals without mouse reporting still work.

Known crossterm issues that the interface must handle (from the tracker, to verify on each platform in U1): a wheel event that arrives as an Escape key press (issue 854), and a press that is dropped when it follows a quick second click (issue 875). The interface treats a lone Escape as cancel only after a short delay with no following input, and it does not depend on press events alone.

Steering. A message typed during a call is queued. The current call finishes, or stops at the next level pass if the user chooses "interrupt and send". The queued message enters the transcript before the next call. Nothing changes inside a reply, because a reply is one block (fact 1).

## Terminal driver: the agent's cursor actions

The agent can operate other terminal programs, the way a person does. This covers clicking, scrolling and typing. The model is text-only, so the driver shows the program's screen as text.

- `term_open` starts a program in a pseudo-terminal of a given size, with a working directory and environment.
- `term_read` returns the screen as a grid of rows, with row and column numbers, and the cursor position. Cells that are not plain text are shown as their character only. The size is limited, and the result says when it is cut.
- `term_click` sends a mouse press and release at a row and column, using the SGR mouse encoding, if the program has enabled mouse reporting. It reports whether the program accepted mouse input.
- `term_scroll` sends wheel events, up or down, at a row and column.
- `term_key` sends a named key (`Enter`, `Tab`, `Up`, `Esc`, `Ctrl-C` and so on). `term_type` is not a separate tool; text goes through `term_key` one character at a time, or through a `text` argument with the same escaping as other tools.
- `term_close` ends the program and its process group.

Approval: the driver is in the `shell` class. Opening a program asks. Reading, clicking and typing into an open program are allowed only in the session where the user approved the open. The interface shows the driver's actions in the transcript.

ConPTY on Windows provides the pseudo-terminal. The pseudo-terminal crate and the VT parser are chosen in U2 after a licence check.

Desktop pointer control (moving and clicking the operating system's pointer outside a terminal) is milestone X1. It needs accessibility trees for each platform (AT-SPI on Linux, UI Automation on Windows, the accessibility API on macOS), and it is off by default. Screenshots are not an option for the text-only model.

## Compaction

A window of P tokens fills fast. With P of 96 to 512 (see "Released NCM models"), compaction is routine, not an exception. It has four layers, applied in this order, and each layer is recorded in the log (D25):

1. **Deterministic pruning** (no model call). Tool results older than the last two turns are replaced by a one-line record: tool name, arguments, status and byte count. The full result stays in the log.
2. **Pinned state** (no model call). Facts the harness knows exactly are kept in structured fields: task, files changed, approvals granted, the current plan, and the open questions. A model writes only the bounded text fields. The pinned state is never summarized away.
3. **Rolling summary** (one worker call). When the packed transcript still exceeds its budget, the oldest unsummarized turns go to a summary worker. The worker's output is capped at L tokens. If the summary is longer than the summary budget, it is summarized again. This is the job the released XSum model is trained for (fact 11), and whether it does the job well for dialogue is unmeasured.
4. **Retrieval** (tools). Older turns stay in the event log. The `search_history` tool returns matching turns, cut to the result limit, and the model can ask for more.

Compaction runs before a call when the packed prompt would exceed the budget. A user can also run `/compact`. The interface shows what was compacted and when, and the user can open the full history of any compacted span.

## Sub-agents

A sub-agent is a separate session, started by the `spawn_agent` tool or by the user.

- **Input.** A role (a named tool subset and instructions), a task text, a pass budget, and a workspace mode.
- **Isolation.** The child has its own transcript and its own window. A `read` child shares the parent's checkout and cannot write. A `write` child works in its own git worktree, created for it, and its changes come back as a patch that the parent shows for approval. The worktree is removed when the child ends, unless the user keeps it.
- **Depth.** A child cannot spawn children (depth 1, D26). The limit prevents an unbounded tree of calls.
- **Budgets.** The pass budget is a hard cap. When it is spent, the child stops and returns what it has, marked `budget_exhausted`.
- **Concurrency.** The number of running children is capped in the configuration. The default is set in H4 after a measurement of the pass time on the target machine.
- **Output.** A child returns status, a summary of at most L tokens, and a list of artifact paths. The parent never receives the child's full transcript unless it calls `search_history` on the child's log.
- **Cancellation.** Cancelling the parent cancels its children. Each child's cancellation is checked between level passes and between operations.

Worker roles are sub-agents with a fixed task and no tool list. A summary worker (compaction) and a translation worker are workers. The orchestrator is the model with the tool list, so it needs P large enough for section 1 of the prompt (D12).

## Side chats

A side chat is a second session in the same process, shown as a tab.

- **Fork.** A side chat starts from a snapshot of the main session's transcript and pinned state. The snapshot is shared and copy-on-write, so forking costs no copy of the transcript.
- **Ask aside.** A one-off question about the current state, which runs as a single call with no tools. Its answer goes to the aside, not to the transcript.
- **Promote.** The user can copy a side chat's summary into the main transcript as a note. The side chat's full history stays in its own log.
- The side chats share the loaded weights. Each has its own scheduling priority, below the main session.

## Sessions, events and persistence

Each session writes an append-only event log, one JSON object per line, in the user's data directory. Events:

- `session_start`: the model's identity (the hash of its `ncm.json` and tensor headers), the approval mode, the working directory, and the configuration in effect.
- `user_message`, `steer`, `ask_answer`, `approval`: the user's inputs and decisions.
- `call_start`: prompt hash, packed token count, `P`, `L`, `K`, sampler settings, seed, and the attempt number.
- `level_done`: level index, elapsed time, and the draft text when drafts are enabled.
- `call_end`: the reply text, whether it was truncated, the parsed items, and the validity result.
- `tool_call`, `tool_result`, `tool_error`.
- `compaction`: the layer, the spans replaced, the token counts before and after, and the summary text.
- `agent_spawn`, `agent_result`, `agent_cancel`.
- `fork`, `promote`.
- `cancel`, `error`.

Resume rebuilds state by replaying the log. Replaying a call with the same seed, prompt and model identity reproduces the same reply on the same machine class, which is the basis of the replay tests. The log is local. Nothing in ncagent sends it anywhere (D34).

## Headless modes

- `ncagent exec <model-dir> --prompt <text>` runs one session to completion and prints the final reply. Approvals fail closed, because nobody can answer them (D30). `--json` prints events, one per line.
- `ncagent serve <model-dir>` speaks JSON-RPC 2.0 over stdio, one message per line. Methods: `session.create`, `session.send`, `session.fork`, `session.cancel`, `approval.respond`, `question.respond`, `session.resume`. Events come as notifications with the same records as the log (D30).

The protocol is versioned in its first message, and the version is part of the log.

## Project instructions

When a session starts, the harness reads `AGENTS.md` from the working directory, and from each parent up to the repository root. The contents go into pinned state under the budget in "Prompt packing". Text beyond the budget is cut, and the cut is recorded (D31). The same rule applies to the user's own configuration, which lives in the user's data directory, and the project's configuration, which lives in `.ncagent/` in the repository.

## Configuration

Settings come from four layers, later ones overriding earlier ones:

1. Built-in defaults.
2. The user's configuration file in the user's configuration directory.
3. The project's `.ncagent/config.toml`.
4. Command-line options.

The configuration names the orchestrator model, the worker models for each role, the approval mode and its default, the number of concurrent sub-agents, the pass budget per turn, the sampler overrides, and the mouse setting. The exact key names are defined with the code in H1 and recorded in the README at that time.

## Permissions and sandbox

Permissions are the approval classes and modes above (D15). The harness does not run a sandbox in v1 (D17). Approval is the only control on shell commands, and the interface says so when a shell command is approved. Sandboxing needs one implementation per operating system, and it is milestone S1, which starts with research. The candidates are Landlock with seccomp on Linux, the sandbox profile facility on macOS, and restricted or AppContainer tokens on Windows. Each candidate is checked before it is chosen.

## Commands

- `ncagent chat <model-dir>`: the terminal interface. `--approval ask|edit|full` sets the mode, and `full` requires the flag.
- `ncagent exec <model-dir> --prompt <text> [--json]`: one headless run.
- `ncagent serve <model-dir>`: JSON-RPC over stdio.
- `ncagent inspect <model-dir>`: checks an export directory. It reads headers only, never weights. It prints the family, `K`, the level sizes, `L`, `P`, the text tokenizer, the parameter count, and whether the model can be an orchestrator.
- `ncagent generate <model-dir> --prompt <text> [--seed <n>]`: one engine call with no tools. For debugging and parity.
- `ncagent convert <released-dir> --out <export-dir>`: converts a released checkpoint folder into an export directory (D7).

Output goes to stdout. Named errors go to stderr. The exit status is 0 on success and 1 on a named error. Option parsing uses `clap`, and each error names the option or file at fault.

## Milestones

Each milestone is done when its check passes and the verification list in AGENTS.md has run on that state.

| ID | Goal | Done when |
|---|---|---|
| A0 | Workspace | `cargo build`, `cargo test` and clippy with `-D warnings` pass on the empty crates. The toolchain pin is installed. `ncagent` dispatches the commands, and each unfinished one returns a named error, never fake output. `.gitignore` covers `target/`, the download folder, export directories, and every weight, tokenizer and pickle type. |
| A1 | `inspect` | Reads a synthetic export directory written by the test. Rejects each broken variant with a named error: a missing tensor, an extra tensor, a wrong shape, an unknown dtype, an unknown `ncm.json` field, and a header over the limit. The `safetensors` crate passes its licence check (D4). |
| T0 | Pins | Revisions and licences of GPT-2, bert-base-uncased and Qwen3-0.6B are recorded. Each released file's size and SHA-256 are written into a fixture from a raw listing. Every crate in the table has its licence and pinned version recorded. |
| T1 | Tokenizers | Each of the five text tokenizers gives the reference's ids on a fixed text set. The `tokenizers` crate is used if it does; otherwise the tokenizer is written here. Golden ids come from the Python helper in `tools/`, run by a developer, and the test uses the committed ids. |
| C1 | Converter | `ncagent convert` writes export directories for xsum, wmt14 and sudoku-easy from the real files. Its inert unpickler produces the 96 keys of the derivation in ncmorph's `wmt14_tokenizer_keys.md`, with the shapes it requires. Any other key or shape is an error. The owner runs the first conversion on the real files. |
| N1 | Engine | Scalar kernels first, then `std::arch` kernels with a scalar test oracle. The prefix is computed once per call and reused across levels, and the output matches a run without the reuse (fact 5). Greedy outputs of xsum, wmt14 and sudoku-easy match the reference's token ids exactly on a fixed prefix (D19). Cancellation is honoured between level passes. |
| N2 | Sampling | Sampler and pick rules match hand-computed cases for temperature, `top_k`, `top_p` and penalties. lm1b and tinystories generate without error. |
| N3 | Guidance | The guided WMT14 output matches the reference at scale 1.0 on a fixed prefix. |
| N4 | Drafts and early stop | Drafts are decoded after each level for xsum. Their quality against the finished reply is measured and recorded in PROJECT.md. If the result is poor, drafts stay off, and the record says so. |
| H1 | Session core | Event log, packing into the window, reply parser, the configuration layers, and `exec` without tools. Tests replay a recorded log to the same state. |
| H2 | Tools and approvals | Read tools, edit operations, the shell with process-group kill, the three approval modes, and the reliability ladder (D16). A declined call returns a tool error and the session continues. |
| H3 | Compaction | The four layers (D25), the summary worker, `search_history`, and `/compact`. A test shows the window stays within P over a long recorded session. |
| H4 | Sub-agents | `spawn_agent`, `await_agent`, `cancel_agent`, the pass budget, worktree isolation, depth 1, and the concurrency cap, with its default measured on the target machine. |
| H5 | Side chats | Fork by snapshot, ask aside, and promote. Forking copies no transcript. |
| U1 | Terminal interface | Mouse wheel, click and drag selection, the level strip, the approval and question widgets, the tabs, and the status line. Keyboard parity is tested for every mouse action. The crossterm issues (854, 875) are reproduced and handled on each platform. |
| U2 | Terminal driver | The six `term_*` tools on Linux, macOS and Windows (ConPTY). The screen is returned as text with row and column numbers. Opening a program needs approval. |
| U3 | Serve | JSON-RPC 2.0 over stdio, versioned, with the methods in "Headless modes". A test drives a session through the protocol. |
| M1 | MCP | Model Context Protocol servers through `rmcp`, with each server's tools shown in the registry and approval class declared per tool. Later. |
| S1 | Sandbox | Research per operating system, then one implementation per system. Later. |
| X1 | Desktop pointer | Research on accessibility trees per platform. Off by default. Later. |
| R1 | Release | Builds for Linux x86_64 and aarch64, macOS arm64 and x86_64, and Windows x86_64 (MSVC), each from one host. The host runs its tests. The other targets are built and not run, and the report says so. |

## Decisions

Each decision names its basis. "Design choice" means the choice comes from this specification, not from the reference or from ncmorph. Design choices are revisited when measurements exist.

- **D1. Rust 1.99.1.** Basis: releases.rs, read 2026-10-26, lists 1.99.1 as stable. The reasons are in "Language and toolchain". ncmorph keeps Zig, and the two projects share no code (D3). Supersedes the Zig decision of the previous revision.
- **D2. Python is not a runtime dependency.** `tools/` holds a Python helper that a developer runs to write golden fixtures from the pinned reference. Tests commit the fixtures, not the helper's output. Basis: the reference is Python, and its outputs are the parity oracle. A user never needs Python. Supersedes the previous revision's Python converter.
- **D3. The export format is ncmorph's, and no code is shared.** Basis: ncmorph's written format (D26). ncmorph's packages are not split into their own repository (ncmorph ROADMAP, Phase 11), so depending on them would pin an unfinished interface.
- **D4. Safetensors through the `safetensors` crate.** It is the Hugging Face reference implementation of the format, at the tag ncmorph pins (v0.8.0), after a licence check at A1. Basis: a second reader would be a second chance for a bug in the one format the models use. Supersedes the previous revision's own reader.
- **D5. Local directories only; the binary reaches no network.** The user downloads checkpoints with the Hugging Face `hf` command. Basis: ncmorph D2.
- **D6. CPU inference in F32, with F32, BF16 and F16 storage.** Every tensor is widened to F32 at load, and kernels use `std::arch` with a scalar oracle. Basis: the released generators are F32, and their sizes match four bytes per parameter. The WMT14 generator is 480,582,539 bytes for about 120 million parameters.
- **D7. Converter in Rust, with an inert unpickler.** `crates/convert` reads torch zip archives and raw pickles. It treats `GLOBAL`, `REDUCE`, `NEWOBJ` and `BUILD` as inert data and executes nothing. It accepts only the storage classes it lists (float, double, half, bfloat16, byte and long), and it extracts tensors only. It checks each file's size and SHA-256 before it reads the file. The owner runs the first conversion. If the reader fails on the real files, the work stops and the owner decides the next step; no fallback that executes Python is added. Basis: candle-core's `pickle.rs` shows that a reader can avoid executing pickle opcodes. This specification writes its own reader and does not depend on that file. The tinystories BPE pickles are read the same way.
- **D8. The pick and sampling follow the config.** Basis: `scripts/eval_shard.py` lines 99 to 124 and `_sample_level_raw`. The paper states none of these settings.
- **D9. The level-0 source follows the config.** Basis: `ncm_facts.md` table A4.
- **D10. Packing order and budgets.** The order and the fractions of P are in "Prompt packing". The fractions are design choices, measured at H1.
- **D11. One call gives at most L text tokens.** A truncated reply continues in a new call, whose prefix holds the partial reply. Basis: one logit row per position (`ncm_facts.md` A2 step 7, C1).
- **D12. Orchestrator versus worker.** A model is an orchestrator only if section 1 of the prompt fits in P/2 and sections 1 and 2 fit in P. Any model can be a worker, whose task fits its window. Basis: the prefix is the only conditioning input (`ncm_facts.md` A3). Design choice for the thresholds.
- **D13. The reply grammar.** Text, `@call` blocks, `@ask` blocks, exact names, one-line `key: value` arguments. Basis: design choice. Released models are not trained on it, and a future model is trained to it. The grammar is chosen because NCM cannot constrain syntax token by token (fact 2).
- **D14. The plain chat template.** Turns are written as `Role: text`, and the prompt ends with `Assistant:`. Assistant text is cut at the first `User:`, `Tool:` or `System:` marker. A model may name another template in `ncm.json`. Basis: design choice.
- **D15. Approval modes and classes.** Classes read, write, shell and agent. Modes `ask` (default), `edit` and `full`. `full` needs the flag. Basis: design choice, modelled on the approval modes of Codex and Claude Code.
- **D16. The reliability ladder.** One sample, then R retries with new seeds, then optional N-best with consensus, then a question to the user. Attempts count toward the turn's pass budget. Basis: NCM cannot correct its own reply within a call (fact 2); the consensus rule is the paper's MBR (paper Section F.4). R and N are design choices, measured at H2.
- **D17. No sandbox in v1.** Approval is the only control. Basis: scope. Sandboxing is S1, and risk R5 records the gap.
- **D18. Text tokenizers by family.** GPT-2 for XSum, bert-base-uncased for LM1B, a custom 8,192-entry BPE for TinyStories, Qwen3-0.6B for WMT14, and 12 symbols for Sudoku. The pre-tokenizer regexes are hand-written to Oniguruma's leftmost-alternative semantics (ncmorph D54) if the `tokenizers` crate does not reproduce the reference. Basis: `ncm_facts.md` section F.
- **D19. Parity is exact on greedy configs.** Token ids from XSum, WMT14 and Sudoku-easy must match the reference exactly on a fixed prefix. Sampled configs are checked against hand-computed cases instead (N2). Basis: those configs are greedy or argmax (`ncm_facts.md` A5).
- **D20. No speed or quality claim without a measurement.** The README's "9 to 31 times faster" is the reference's claim, and this specification does not repeat it. Basis: ncmorph's scope rule.
- **D21. Licence.** ncagent is Apache-2.0. Each export's `NOTICE` carries the reference's MIT notice, with the copyright line `Copyright (c) 2025 hermann kumbong` as read from its LICENSE at the pinned commit (ncmorph research), and the licence of each text tokenizer, read from its Hugging Face API response in T0. Each crate in the table passes a licence check when it is added. Basis: ncmorph's "Licences and notices".
- **D22. The header.** Rust, C and C++ use the owner's block form exactly. Python and shell use `#` lines. Data, configuration and Markdown carry none. Basis: the owner's instruction, and ncmorph decision D42.
- **D23. Scope.** Included: everything in "Scope", including `serve`. Excluded: training, GPU and downloads. Basis: scope.
- **D24. Sessions are logs.** The append-only event log is the source of truth for a session. Resume replays it. Each call records its seed, prompt hash and model identity. Basis: design choice, required for resume, replay tests and audit.
- **D25. Compaction in four layers, deterministic first.** Pruning, pinned state, rolling summary, retrieval, in that order. The summary uses a worker, and its output is capped at L. Basis: P is small (fact 4), and the XSum model's training matches a short summary (fact 11). The layer order is a design choice.
- **D26. Sub-agents at depth 1 with pass budgets and worktree isolation.** Basis: design choice. Depth 1 bounds the call tree, and a worktree keeps parallel writers from colliding.
- **D27. Side chats fork by snapshot.** Basis: design choice. A copy-on-write snapshot makes a fork cheap.
- **D28. The terminal interface uses ratatui over crossterm 0.29.0.** It supports the mouse events in "Terminal interface", with keyboard parity. Basis: crossterm's MouseEventKind (docs.rs, version 0.29.0), and the issues listed there.
- **D29. The agent's cursor actions go through a terminal driver with text screens.** No pixels. The driver is text-only, and desktop pointer control is later and off by default (X1). Basis: fact 10.
- **D30. Headless modes.** `exec` refuses every action that would ask. `serve` speaks JSON-RPC 2.0 over stdio, versioned. Basis: design choice. Fail-closed is the safe default when no one can answer.
- **D31. Project instructions come from `AGENTS.md`.** The harness reads it from the working directory up to the repository root, within the budget. Basis: the file already governs this project's agents, and the same convention lets other projects use the harness.
- **D32. MCP through `rmcp` later.** The official SDK implements MCP spec 2026-07-28 and is compatible with 2025-11-25, according to its README (github.com/modelcontextprotocol/rust-sdk). Its licence is checked before M1. Basis: the SDK exists, so writing an MCP client here would duplicate it.
- **D33. Scheduling.** Foreground sessions have priority over sub-agents and side chats on the compute pool. Basis: design choice, so the user's session stays responsive while children run.
- **D34. No telemetry.** The event log stays on the user's machine. Basis: design choice, and the owner's privacy stance in the working rules.
- **D35. The tool registry is the only list of tools.** Each tool appears once, with its arguments, limits and class. The protocol section and the approval rules read from it. Basis: design choice, to keep the model's prompt, the parser and the approvals consistent.

Superseded decisions from the previous revision: Zig 0.17.0 as the language (D1), the Python converter (D2 and D7), and the project's own safetensors reader (D4). Each is replaced above, and the old text is not kept.

## Risks

- **R1. Agent quality is unproven.** No released model follows instructions or calls tools. The harness can be correct and still produce no useful agent behaviour. A model for this job must be trained to the protocol, and ncmorph, which converts open-weight LLMs into NCM models, is one route to one.
- **R2. The window is small.** P of 96 cannot hold the tool list of section 1 (D12). Tools are off for such a model, so it is a worker only. An orchestrator needs a larger P, which is a training-time choice.
- **R3. Replies are short and fixed-length.** Every reply costs L positions, and a file change needs several calls (fact 3). Long tasks take many passes, and the pass budget bounds them.
- **R4. Reply syntax is not guaranteed.** Positions are sampled in parallel (fact 2). The reliability ladder (D16) and its pass cost are the only protection, and the failure rate is measured at H2.
- **R5. Shell commands have no sandbox.** Approval is the only control (D17). `full` mode accepts that every command runs unchecked inside the working directory.
- **R6. The released files may not unpickle cleanly.** The inert reader (D7) has not been run on the real files. If it fails, the conversion stops, and the owner decides the next step.
- **R7. Numeric parity.** The engine computes in F32 on the CPU, and the reference runs under PyTorch. Greedy outputs can differ at near-ties in the logits. Each mismatch is measured at N1, and no tolerance is set before then.
- **R8. Terminal behaviour varies.** Mouse reporting differs between terminals, the crossterm issues (854, 875) are open, and mintty and MSYS are not supported for mouse (U1). Keyboard parity covers the gap.
- **R9. Compaction may lose facts.** The summary worker is a model. Pinned state (layer 2) and the event log (layer 4) are the protection, and the layer's quality is measured in H3.
- **R10. Sub-agents multiply cost.** Each child spends passes from the same machine. The concurrency cap and the pass budgets bound it.
- **R11. Rust naming friction.** The two allowed lints (see "Language and toolchain") are the only exceptions to the compiler's defaults. Clippy runs with `-D warnings` otherwise.
- **R12. Reference README and listing disagree on OpenWebText.** ncagent follows the listing (see "Released NCM models").
- **R13. The toolchain pin is one release ahead of the local machine.** The machine checked on 2026-10-26 has rustc 1.98.0. The pin is installed in A0.
- **R14. Crate licences and maintenance.** Each crate passes a licence check when it is added, and a crate that stops being maintained is replaced by a decision in this file.

## References

- Paper: [arXiv:2610.14162](https://arxiv.org/abs/2610.14162), Next-Concept Modeling.
- Reference code: [github.com/HazyResearch/ncm](https://github.com/HazyResearch/ncm) at `c70274273f489abc73832b2b98584a8483b6b203` (MIT).
- Checkpoints: [huggingface.co/hazyresearch/ncm-checkpoints](https://huggingface.co/hazyresearch/ncm-checkpoints) at `d342ce8aaea58830eef3b8ad473664673e41f692`.
- Datasets: [huggingface.co/hazyresearch/ncm-datasets](https://huggingface.co/hazyresearch/ncm-datasets).
- Sibling project: [github.com/Kayyo321/ncmorph](https://github.com/Kayyo321/ncmorph). It defines the export format and holds the research record cited above.
- Rust releases: [releases.rs](https://releases.rs/), read 2026-10-26.
- crossterm 0.29.0, `MouseEventKind`: [docs.rs/crossterm](https://docs.rs/crossterm/latest/crossterm/event/enum.MouseEventKind.html).
- Codex CLI: [github.com/openai/codex](https://github.com/openai/codex), and its [CLI documentation](https://developers.openai.com/codex/cli.md).
- Official Model Context Protocol Rust SDK: [github.com/modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk).
- candle-core's pickle reader, the design reference for D7: [github.com/huggingface/candle](https://github.com/huggingface/candle/blob/main/candle-core/src/pickle.rs).
- libvaxis, considered and rejected for D1: [github.com/rockorager/libvaxis](https://tangled.org/rockorager.dev/libvaxis).
- safetensors format: [github.com/huggingface/safetensors](https://github.com/huggingface/safetensors).
