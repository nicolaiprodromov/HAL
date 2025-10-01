use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// HAL — run natural-language pseudo-code through a three-agent pipeline.
#[derive(Debug, Parser)]
#[command(name = "hal", version, about, long_about = None)]
struct Cli {
    /// Detached mode. Bypass the TUI; the process behaves like running
    /// the script directly — only the container's stdout/stderr reach
    /// this process's stdout/stderr. Pipeline events are suppressed.
    #[arg(short = 'd', long = "direct", global = true)]
    direct: bool,

    /// Force a fresh Interpreter + Architect rebuild even if the source
    /// hash matches the cached `build.lock`. Use this after tweaking
    /// `.hal/config.toml` or when you just want to re-roll the agents.
    #[arg(short = 'f', long = "force", global = true)]
    force: bool,

    /// Enable stderr logging in addition to the rolling file log.
    /// Pairs well with `-d` for script-style debugging.
    #[arg(short = 'v', long = "verbose", global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Command>,

    /// Shorthand for `hal run <FILE> [ARGS...]` when no subcommand is given.
    #[arg(value_name = "FILE")]
    file: Option<PathBuf>,

    /// Arguments forwarded to the running script (after the file path).
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the Interpreter + Architect and materialize the artifact bundle.
    Build {
        /// Path to the `.hal` source file.
        file: PathBuf,
    },
    /// Execute the artifact bundle (builds first if missing or stale).
    ///
    /// Anything after `<FILE>` is forwarded to the script as
    /// command-line arguments, exactly like `python script.py arg1 arg2`.
    Run {
        /// Path to the `.hal` source file.
        file: PathBuf,

        /// Arguments forwarded to the running script.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    // Load .env as early as possible so downstream config can see it.
    let _ = dotenvy::dotenv();
    // Also try HAL/.env relative to the current directory (dev-container case).
    let _ = dotenvy::from_filename("HAL/.env");

    let cli = Cli::parse();

    // Initialise logging BEFORE anything else that could fail, so crashes
    // during setup still end up in `~/.hal/logs/hal.log`.
    let _log_guard = hal::logging::init(cli.verbose).ok();
    tracing::info!(
        target: "hal::cli",
        direct = cli.direct,
        force = cli.force,
        verbose = cli.verbose,
        command = ?cli.command,
        file = ?cli.file,
        args = ?cli.args,
        "parsed CLI"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let result = runtime.block_on(async_main(cli));
    if let Err(e) = &result {
        tracing::error!(target: "hal::cli", error = %format!("{e:#}"), "hal exiting with error");
    } else {
        tracing::info!(target: "hal::cli", "hal exited cleanly");
    }
    result
}

async fn async_main(cli: Cli) -> anyhow::Result<()> {
    match (cli.command, cli.file) {
        (Some(Command::Build { file }), _) => {
            hal::commands::build::run(&file, cli.direct, cli.force).await
        }
        (Some(Command::Run { file, args }), _) => {
            hal::commands::run::run(&file, cli.direct, cli.force, &args).await
        }
        (None, Some(file)) => {
            hal::commands::run::run(&file, cli.direct, cli.force, &cli.args).await
        }
        (None, None) => {
            // No subcommand, no file — emit help.
            use clap::CommandFactory;
            Cli::command().print_help()?;
            println!();
            Ok(())
        }
    }
}
