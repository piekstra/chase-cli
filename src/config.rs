//! Non-secret settings in `${XDG_CONFIG_HOME:-~/.config}/chse/config.json`
//! (SPEC v1 §1.2), or wherever `--config <PATH>` points.
//!
//! There is nothing secret to keep here — this CLI holds no credential (see
//! `docs/api.md`) — so the whole file is plain user preference.

use pk_cli_config::ConfigStore;
use pk_cli_core::{output, CliError, CommonArgs};
use serde::{Deserialize, Serialize};

/// Known keys, kept in one place so `set`/`unset` and the error message can't
/// drift apart.
const KNOWN_KEYS: [&str; 1] = ["inbox"];

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Default directory for `import scan` — typically the Drive inbox that
    /// Chase downloads land in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbox: Option<String>,
}

#[derive(clap::Subcommand, Debug)]
pub enum ConfigCmd {
    /// Print the resolved config file path.
    Path,
    /// Show the effective configuration.
    Show,
    /// Set a config key (e.g. `config set inbox ~/Drive/inbox`).
    Set { key: String, value: String },
    /// Remove a config key.
    Unset { key: String },
}

fn unknown(key: &str) -> CliError {
    CliError::Usage(format!(
        "unknown config key `{key}` (known: {})",
        KNOWN_KEYS.join(", ")
    ))
}

pub fn run(cmd: &ConfigCmd, store: &ConfigStore, common: &CommonArgs) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Path => {
            println!("{}", store.path()?.display());
            Ok(())
        }
        ConfigCmd::Show => {
            let cfg: Config = store.load()?;
            let v = serde_json::to_value(&cfg).unwrap_or_default();
            if common.json {
                output::json(&v);
            } else {
                output::render(&v);
            }
            Ok(())
        }
        ConfigCmd::Set { key, value } => {
            let mut cfg: Config = store.load()?;
            match key.as_str() {
                "inbox" => cfg.inbox = Some(value.clone()),
                other => return Err(unknown(other)),
            }
            store.save(&cfg)
        }
        ConfigCmd::Unset { key } => {
            let mut cfg: Config = store.load()?;
            match key.as_str() {
                "inbox" => cfg.inbox = None,
                other => return Err(unknown(other)),
            }
            store.save(&cfg)
        }
    }
}
