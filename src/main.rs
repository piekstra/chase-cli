//! `chse` — Chase CLI for the piekstra family (conforms to piekstra-cli/1).
//!
//! **Offline-first, and deliberately so.** There is no portal client here:
//! JPMorgan Chase publishes no personal-banking API a customer can self-serve
//! against, and finished eliminating third-party screen scraping in 2022. So
//! this CLI's job is to make the export Chase *does* hand you — CSV activity
//! downloads — into clean, schema-tagged family DTOs. `docs/api.md` records
//! the full reasoning, the research behind it, and the realistic future paths.

mod commands;
mod config;
mod dto;
mod format;
mod parser;
mod scan;
mod value;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use pk_cli_auth::{AuthMethod, AuthStatus};
use pk_cli_config::ConfigStore;
use pk_cli_core::info::{AuthInfo, CliInfo};
use pk_cli_core::{output, CliError, CommonArgs};
use pk_cli_selfupdate::{SelfUpdateArgs, Updater};

use commands::import::ImportCmd;
use config::ConfigCmd;

const BIN: &str = "chse";
const REPO: &str = "piekstra/chase-cli";

/// Chase card and bank activity, from Chase's own CSV exports.
#[derive(Parser, Debug)]
#[command(name = BIN, version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,

    /// Override the config file location (SPEC v1 §1.1).
    #[arg(long = "config", global = true, value_name = "PATH")]
    config_path: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Parse and discover Chase CSV exports.
    #[command(subcommand)]
    Import(ImportCmd),
    /// Credential state. This CLI needs none — see `docs/api.md`.
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Non-secret settings.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Update to the latest release from GitHub.
    SelfUpdate(SelfUpdateArgs),
    /// Print a shell completion script.
    Completions { shell: Shell },
    /// Machine-readable capability discovery (cli-info/v1).
    Info,
}

#[derive(Subcommand, Debug)]
enum AuthCmd {
    /// Report credential state (auth-status/v1). Always `required: false`.
    Status,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        std::process::exit(output::fail(&e, cli.common.json));
    }
}

fn run(cli: &Cli) -> Result<(), CliError> {
    let store = ConfigStore::new(BIN).with_override(cli.config_path.clone());

    match &cli.command {
        Command::Import(cmd) => commands::import::run(cmd, &store, &cli.common),
        Command::Auth(AuthCmd::Status) => {
            // `required: false` + `method: none` is the spec's credential-free
            // shape (SPEC §1.2): drivers read "usable now" without special-
            // casing this CLI. `authenticated: true` because every command
            // works — there is nothing to log in to.
            AuthStatus::new(false, true, AuthMethod::None).emit(cli.common.json);
            Ok(())
        }
        Command::Config(cmd) => config::run(cmd, &store, &cli.common),
        Command::SelfUpdate(args) => Updater {
            repo: REPO.into(),
            binary: BIN.into(),
            target: env!("BUILD_TARGET").into(),
            current: env!("CARGO_PKG_VERSION").into(),
        }
        .run(args, cli.common.json, cli.common.quiet),
        Command::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), BIN, &mut std::io::stdout());
            Ok(())
        }
        Command::Info => {
            let info = CliInfo::new(
                BIN,
                env!("CARGO_PKG_VERSION"),
                &format!("https://github.com/{REPO}"),
                AuthInfo {
                    required: false,
                    method: "none".into(),
                    login_hint: None,
                },
                &["import"],
            );
            output::json(&serde_json::to_value(&info).unwrap());
            Ok(())
        }
    }
}
