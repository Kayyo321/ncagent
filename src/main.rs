/*
 * Ko, (Baczynski-Bruce) 2026; Apache 2.0
*/
#![allow(non_camel_case_types, non_upper_case_globals)]

//! The ncagent binary. This file dispatches commands and nothing else
//! (PROJECT.md, "Architecture"). Commands whose milestone is not done return a
//! named error and print no output on stdout.

use std::fmt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use harness::api::SessionError;
use harness::approval::ApprovalMode;
use harness::backend::BackendError;
use harness::runtime::{
    DefaultSeed, MaxAttempts, PassBudget, controller_runtime_t, runtime_config_t, system_clock,
};
use harness::store::{memory_store_t, session_store_t};
use tui::{TuiError, tui_options_t};

#[derive(Parser, Debug)]
#[command(
    name = "ncagent",
    version,
    about = "Command-line agent for Next-Concept Modeling (NCM) models"
)]
struct cli_t {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Terminal interface (milestone U1).
    Chat {
        /// Export directory of the model.
        model_dir: PathBuf,
        /// Approval mode. `full` must be selected explicitly.
        #[arg(long, value_enum, default_value_t = ApprovalArg::Ask)]
        approval: ApprovalArg,
    },
    /// One headless run (milestone H1).
    Exec {
        /// Export directory of the model.
        model_dir: PathBuf,
        /// Prompt text.
        #[arg(long)]
        prompt: String,
        /// Print events, one JSON object per line.
        #[arg(long)]
        json: bool,
    },
    /// JSON-RPC 2.0 over stdio (milestone U3).
    Serve {
        /// Export directory of the model.
        model_dir: PathBuf,
    },
    /// Check an export directory, reading headers only (milestone A1).
    Inspect {
        /// Export directory of the model.
        model_dir: PathBuf,
    },
    /// One engine call with no tools (milestone N1).
    Generate {
        /// Export directory of the model.
        model_dir: PathBuf,
        /// Prompt text.
        #[arg(long)]
        prompt: String,
        /// Sampling seed.
        #[arg(long)]
        seed: Option<u64>,
    },
    /// Convert a released checkpoint folder into an export directory (milestone C1).
    Convert {
        /// Folder of a released checkpoint.
        released_dir: PathBuf,
        /// Export directory to write.
        #[arg(long)]
        out: PathBuf,
    },
    /// DEMO: the terminal UI against a scripted simulator (owner exception O1, milestone P1).
    /// No model runs and no tool executes.
    Demo {
        /// Capabilities the simulator reports (limits differ between profiles).
        #[arg(long, value_enum, default_value_t = ProfileArg::Standard)]
        profile: ProfileArg,
        /// Simulated timing. `instant` also turns the animation off.
        #[arg(long, value_enum, default_value_t = SpeedArg::Normal)]
        speed: SpeedArg,
        /// Approval mode. `full` must be selected explicitly.
        #[arg(long, value_enum, default_value_t = ApprovalArg::Ask)]
        approval: ApprovalArg,
        /// Session seed.
        #[arg(long, default_value_t = DefaultSeed)]
        seed: u64,
        /// Directory for demo state: sessions in `sessions/`, folders in `folders.json`.
        /// Defaults to the platform's local data directory.
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Keep no demo state on disk. Sessions and folders are kept in memory only.
        #[arg(long)]
        ephemeral: bool,
        /// Do not capture the mouse.
        #[arg(long)]
        no_mouse: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum ApprovalArg {
    Ask,
    Edit,
    Full,
}

impl From<ApprovalArg> for ApprovalMode {
    fn from(arg: ApprovalArg) -> Self {
        match arg {
            ApprovalArg::Ask => ApprovalMode::Ask,
            ApprovalArg::Edit => ApprovalMode::Edit,
            ApprovalArg::Full => ApprovalMode::Full,
        }
    }
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum ProfileArg {
    Standard,
    Compact,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum SpeedArg {
    Normal,
    Fast,
    Instant,
}

#[derive(Debug)]
enum CliError {
    NotImplemented {
        command: &'static str,
        milestone: &'static str,
        detail: String,
    },
    Backend(BackendError),
    Session(SessionError),
    Tui(TuiError),
    Setup(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::NotImplemented {
                command,
                milestone,
                detail,
            } => write!(
                f,
                "`{command}` is not implemented yet (milestone {milestone}); {detail}"
            ),
            CliError::Backend(err) => write!(f, "{err}"),
            CliError::Session(err) => write!(f, "{err}"),
            CliError::Tui(err) => write!(f, "{err}"),
            CliError::Setup(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Start the terminal UI against the demo backend (O1, D36).
fn run_demo(
    profile: ProfileArg,
    speed: SpeedArg,
    approval: ApprovalArg,
    seed: u64,
    state_dir: Option<PathBuf>,
    ephemeral: bool,
    no_mouse: bool,
) -> Result<(), CliError> {
    let profile_kind = match profile {
        ProfileArg::Standard => demo::profile::ProfileKind::Standard,
        ProfileArg::Compact => demo::profile::ProfileKind::Compact,
    };
    let timing = match speed {
        SpeedArg::Normal => demo::timing::timing_t::normal(),
        SpeedArg::Fast => demo::timing::timing_t::fast(),
        SpeedArg::Instant => demo::timing::timing_t::instant(),
    };
    let (backend, tools) = demo::build(profile_kind, timing);

    // The state directory holds the sessions and `folders.json` (D40, D50). `None` keeps both in
    // memory.
    let (store, state_dir, state_note): (Arc<dyn session_store_t>, Option<PathBuf>, String) =
        if ephemeral {
            (
                Arc::new(memory_store_t::new()),
                None,
                "demo state: kept in memory only (--ephemeral)".to_string(),
            )
        } else {
            let dir = match state_dir {
                Some(dir) => dir,
                None => demo::paths::default_state_dir().map_err(CliError::Setup)?,
            };
            let note = format!("demo state: {}", dir.display());
            (
                Arc::new(demo::store::file_store_t::new(dir.clone())),
                Some(dir),
                note,
            )
        };

    let cwd = std::env::current_dir()
        .map_err(|err| CliError::Setup(format!("cannot read the working directory: {err}")))?;
    let config = runtime_config_t {
        cwd,
        approval: approval.into(),
        session_seed: seed,
        max_attempts: MaxAttempts,
        pass_budget: PassBudget,
        version: env!("CARGO_PKG_VERSION").to_string(),
        notice: Some(demo::Notice.to_string()),
        label: Some(demo::Label.to_string()),
        state_note: Some(state_note),
        hints: demo::hints(),
        clock: system_clock,
    };
    let runtime =
        controller_runtime_t::new(backend, tools, store, config).map_err(CliError::Session)?;
    let runtime = match state_dir {
        Some(dir) => runtime.with_organization_store(Arc::new(
            demo::organize::file_organization_store_t::new(dir),
        )),
        None => runtime,
    };
    let options = tui_options_t {
        animate: speed != SpeedArg::Instant,
        mouse: !no_mouse,
        color: None,
    };
    tui::run(Arc::new(runtime), options).map_err(CliError::Tui)
}

fn run(cli: cli_t) -> Result<(), CliError> {
    match cli.command {
        Command::Chat {
            model_dir,
            approval,
        } => {
            // The single plug-in point for a real model: `open_export` returns the engine once
            // milestones A1 and N1 are done. Today it returns a named not-implemented error.
            let _backend = ncm::engine::open_export(&model_dir).map_err(CliError::Backend)?;
            Err(CliError::NotImplemented {
                command: "chat",
                milestone: "H1",
                detail: format!(
                    "the model opened, but production sessions need the event log and the session store; approval mode {approval:?} was not applied"
                ),
            })
        }
        Command::Demo {
            profile,
            speed,
            approval,
            seed,
            state_dir,
            ephemeral,
            no_mouse,
        } => run_demo(
            profile, speed, approval, seed, state_dir, ephemeral, no_mouse,
        ),
        Command::Exec {
            model_dir,
            prompt,
            json,
        } => Err(CliError::NotImplemented {
            command: "exec",
            milestone: "H1",
            detail: format!(
                "the model directory {} was not opened, the {}-byte prompt was not sent and json output is {}",
                model_dir.display(),
                prompt.len(),
                if json { "on" } else { "off" }
            ),
        }),
        Command::Serve { model_dir } => Err(CliError::NotImplemented {
            command: "serve",
            milestone: "U3",
            detail: format!("the model directory {} was not opened", model_dir.display()),
        }),
        Command::Inspect { model_dir } => Err(CliError::NotImplemented {
            command: "inspect",
            milestone: "A1",
            detail: format!("the model directory {} was not read", model_dir.display()),
        }),
        Command::Generate {
            model_dir,
            prompt,
            seed,
        } => Err(CliError::NotImplemented {
            command: "generate",
            milestone: "N1",
            detail: format!(
                "the model directory {} was not opened, the {}-byte prompt was not sent and the seed is {}",
                model_dir.display(),
                prompt.len(),
                seed.map_or_else(|| "unset".to_string(), |value| value.to_string())
            ),
        }),
        Command::Convert { released_dir, out } => Err(CliError::NotImplemented {
            command: "convert",
            milestone: "C1",
            detail: format!(
                "{} was not read and {} was not written",
                released_dir.display(),
                out.display()
            ),
        }),
    }
}

fn main() -> ExitCode {
    let cli = match cli_t::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            // The specification gives status 0 on success and 1 on a named error,
            // so usage errors exit with 1 rather than clap's default of 2.
            return if err.use_stderr() {
                eprint!("{err}");
                ExitCode::from(1)
            } else {
                print!("{err}");
                ExitCode::SUCCESS
            };
        }
    };
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("ncagent: error: {err}");
            ExitCode::from(1)
        }
    }
}
