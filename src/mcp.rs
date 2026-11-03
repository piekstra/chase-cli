//! `chse mcp`: the user's Chase CSV exports as MCP tools (cli-common DESIGN.md §1.9,
//! the MCP profile), for Claude Code, Claude Desktop and any MCP client.
//!
//! Two read-only tools over the parser the commands use: `chase_transactions` and
//! `chase_exports`. There is no Chase account behind them, only files the user
//! downloaded, so nothing here can move money or change anything anywhere.
//!
//! # A model cannot name a file
//!
//! `import csv <FILE>` reads whatever path it is given, which is fine for a person
//! and is not fine for a model: a path argument would let anything a model was told
//! read any CSV on the computer. So no tool takes a path. They read the `inbox` the
//! user configured, or `$CHSE_INBOX`, on each call. A `file` argument is compared
//! with the names found there and never opened. No result carries the inbox's own
//! path, and a run of 8 or more digits in text from a file is masked, since a
//! checking export's descriptions can carry account numbers.
//!
//! Listing the tools reads no file and no credential. The only secret this binary
//! can touch is the bearer token of `mcp --http`, held by `pk-cli-mcp`. So nothing
//! here takes a lock or renews anything, and a call reads every file again.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use pk_cli_config::ConfigStore;
use pk_cli_core::{dates, CliError, Money, RangeArgs};
use pk_cli_mcp::{
    parse_args, ListArgs, Page, PageRequest, Server, ToolDef, ToolError, ToolOutput, Toolset,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::import::display_path;
use crate::config::Config;
use crate::dto::Transaction;
use crate::{parser, scan, value, BIN};

pub const TRANSACTIONS_TOOL: &str = "chase_transactions";
pub const EXPORTS_TOOL: &str = "chase_exports";

/// `chse config set inbox`, as the one setup these tools have. The server's standard
/// instructions take it as what to ask of the user when access is missing.
const SET_INBOX: &str = "chse config set inbox <folder>";

/// The fix for a folder that came from the config.
const POINT_AT_A_FOLDER: &str = "Ask the user to download their activity from chase.com as CSV \
                                 into a folder and run `chse config set inbox <folder>` in a \
                                 terminal with the folder's full path, then call the tool again";

/// The fix for a folder that came from `$CHSE_INBOX`, which the Claude Desktop extension
/// sets. `config set` changes nothing while that variable is set.
const CHANGE_THE_SETTING: &str = "Ask the user to choose the folder again, by its full path, in \
                                  this server's settings in their client (for Claude Desktop, \
                                  the extension's settings), then call the tool again";

/// The fix when the inbox is there and holds no export.
const DOWNLOAD_ONE: &str = "Ask the user to download their activity from chase.com as CSV into \
                            the inbox folder, then call the tool again";

/// A digit run this long is the shape of an account, card or routing number.
const LONG_DIGIT_RUN: usize = 8;

/// A folder to read, and how a person changes it if it is no use.
pub struct Located {
    pub dir: PathBuf,
    pub fix: &'static str,
}

/// Where the tools read exports from. Asked once per call.
pub trait Inbox {
    fn dir(&self) -> Result<Located, ToolError>;
}

/// The inbox a person chose: `$CHSE_INBOX`, which the Claude Desktop extension sets
/// from the folder picked in its settings, or else the config's `inbox`.
pub struct ConfiguredInbox {
    config_path: Option<PathBuf>,
}

impl ConfiguredInbox {
    pub fn new(config_path: Option<PathBuf>) -> Self {
        Self { config_path }
    }
}

/// A relative path would mean whatever folder the client started the server in, so
/// the same setting would read different files from project to project. A quoted
/// `~/Drive/inbox` lands here too, since nothing expands it. The value is not repeated.
fn absolute(dir: PathBuf, fix: &'static str) -> Result<Located, ToolError> {
    if dir.is_absolute() {
        Ok(Located { dir, fix })
    } else {
        Err(ToolError::not_found("the inbox folder is not a full path").with_fix(fix))
    }
}

impl Inbox for ConfiguredInbox {
    fn dir(&self) -> Result<Located, ToolError> {
        if let Some(dir) = std::env::var_os("CHSE_INBOX").filter(|d| !d.is_empty()) {
            return absolute(PathBuf::from(dir), CHANGE_THE_SETTING);
        }
        let store = ConfigStore::new(BIN).with_override(self.config_path.clone());
        let cfg: Config = store.load().map_err(|_| {
            ToolError::other("the chse config file cannot be read")
                .with_fix("Ask the user to run `chse config show` in a terminal and repair it")
        })?;
        let dir = cfg
            .inbox
            .filter(|d| !d.trim().is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                ToolError::not_found("no inbox folder is configured").with_fix(POINT_AT_A_FOLDER)
            })?;
        absolute(dir, POINT_AT_A_FOLDER)
    }
}

/// The toolset `chse mcp` serves.
pub struct ChaseTools<I: Inbox> {
    inbox: I,
}

impl<I: Inbox> ChaseTools<I> {
    pub fn new(inbox: I) -> Self {
        Self { inbox }
    }
}

/// Mask each run of [`LONG_DIGIT_RUN`] or more digits to its last four.
fn mask_digit_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= LONG_DIGIT_RUN {
            out.push_str("****");
            out.push_str(&run[run.len() - 4..]);
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// A file's name as a result shows it. Chase stamps dates into its export names, and an
/// 8-digit date is no account number, so those stay: masked, every export would look
/// alike. Any other long run is masked.
fn shown_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(start) = rest.find(|c: char| c.is_ascii_digit()) {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        let run = &tail[..end];
        if is_date_stamp(run) {
            out.push_str(run);
        } else {
            out.push_str(&mask_digit_runs(run));
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn is_date_stamp(run: &str) -> bool {
    run.len() == 8
        && (run.starts_with("19") || run.starts_with("20"))
        && dates::parse_iso(&format!("{}-{}-{}", &run[..4], &run[4..6], &run[6..])).is_ok()
}

/// One Chase-shaped file. `parsed` is `None` when it could not be read to the end.
struct Export {
    /// Relative to the inbox, masked, and unique among the exports read.
    name: String,
    parsed: Option<parser::ParsedFile>,
}

/// What the inbox holds.
struct Reading {
    csv_files: usize,
    exports: Vec<Export>,
}

/// Read every Chase-shaped CSV in the folder. Errors name the inbox, not its path: where
/// a person keeps their files is no part of an answer about their spending.
///
/// A file that cannot be opened may be an export, so it is kept as one that could not be
/// read, and an answer says it is missing. A stray binary file with a `.csv` name is
/// just not Chase's. Masking can give two files one name, and `file` has to select one,
/// so a later one gets a suffix in the sorted order `find_csvs` keeps.
fn read_inbox(inbox: &Located, recursive: bool) -> Result<Reading, ToolError> {
    let fix = inbox.fix;
    let dir = inbox.dir.as_path();
    let candidates = scan::find_csvs(dir, recursive).map_err(|e| match e {
        CliError::NotFound(_) => {
            ToolError::not_found("the inbox folder does not exist").with_fix(fix)
        }
        CliError::Usage(_) => {
            ToolError::not_found("the inbox is a file, not a folder").with_fix(fix)
        }
        _ => ToolError::other("the inbox folder cannot be read").with_fix(fix),
    })?;
    let mut exports = Vec::new();
    let mut taken = HashSet::new();
    for path in &candidates {
        let readable = match scan::probe(path) {
            Ok(Some(_)) => true,
            Ok(None) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => continue,
            Err(_) => false,
        };
        let shown = shown_name(&display_path(path, dir));
        let mut name = shown.clone();
        let mut nth = 1;
        while !taken.insert(name.clone()) {
            nth += 1;
            name = format!("{shown}~{nth}");
        }
        exports.push(Export {
            name,
            parsed: readable.then(|| parser::parse_file(path).ok()).flatten(),
        });
    }
    Ok(Reading {
        csv_files: candidates.len(),
        exports,
    })
}

fn non_empty(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

/// `since`/`until` as a checked range. Comparison downstream is on ISO text, so what
/// goes in has to be ISO.
fn range(since: Option<&str>, until: Option<&str>) -> Result<RangeArgs, ToolError> {
    let parse = |name: &str, v: Option<&str>| {
        v.map(|d| {
            dates::parse_iso(d).map(dates::fmt_iso).map_err(|_| {
                ToolError::usage(format!("{name} must be a date as YYYY-MM-DD, got `{d}`"))
            })
        })
        .transpose()
    };
    let (since, until) = (parse("since", since)?, parse("until", until)?);
    if let (Some(s), Some(u)) = (&since, &until) {
        if s > u {
            return Err(ToolError::usage(format!(
                "since ({s}) is after until ({u})"
            )));
        }
    }
    Ok(RangeArgs {
        limit: None,
        since,
        until,
    })
}

// ---- chase_transactions ---------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TransactionsArgs {
    /// First day, YYYY-MM-DD. Default: the earliest in the exports.
    #[serde(default)]
    pub since: Option<String>,
    /// Last day, YYYY-MM-DD. Default: the latest in the exports.
    #[serde(default)]
    pub until: Option<String>,
    /// Only descriptions containing this, ignoring case.
    #[serde(default)]
    pub description: Option<String>,
    /// Only this Chase category, ignoring case, such as Groceries.
    #[serde(default)]
    pub category: Option<String>,
    /// Only this kind: charge, payment, refund, fee, adjustment, debit, credit, check
    /// or deposit.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only this export, by the name chase_exports gives it.
    #[serde(default)]
    pub file: Option<String>,
    /// Also read the inbox's subfolders. Default: false.
    #[serde(default)]
    pub recursive: bool,
    #[serde(flatten)]
    pub list: ListArgs,
}

#[derive(Serialize)]
struct TransactionsFilter<'a> {
    since: Option<&'a str>,
    until: Option<&'a str>,
    description: Option<&'a str>,
    category: Option<&'a str>,
    kind: Option<&'a str>,
    file: Option<&'a str>,
    recursive: bool,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct TransactionItem {
    pub date: String,
    pub description: String,
    /// Charges positive; payments and refunds negative.
    pub amount: String,
    pub kind: String,
    /// Chase's own, on card exports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chase_type: Option<String>,
    /// The running balance, on deposit exports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_number: Option<String>,
    /// The export this row came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TransactionsOut {
    /// Rows matching this call's filters, across every page.
    pub matched: usize,
    /// The sum of amount over those rows, across every page. Payments and refunds
    /// count against charges, so filter by kind for spending alone.
    pub total_amount: String,
    /// The earliest date in any export read, whatever the filters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_from: Option<String>,
    /// The latest date in any export read. Nothing later is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_through: Option<String>,
    /// False when a row or a whole export could not be read, so rows are missing.
    pub complete: bool,
    /// Rows identical to one in another export, in matched. They may be one
    /// transaction downloaded twice, or two that look alike on different accounts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possible_duplicates: Option<usize>,
    /// What this answer could not cover.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub page: Page<TransactionItem>,
}

/// The same row as another export has it.
type RowKey = (String, String, String);

fn row_key(t: &Transaction) -> RowKey {
    (
        t.date.clone(),
        t.amount.amount.clone(),
        t.description.clone(),
    )
}

fn mask_text(t: Option<String>) -> Option<String> {
    t.map(|m| mask_digit_runs(&m))
}

fn transaction_item(t: Transaction, file: &str, detailed: bool) -> TransactionItem {
    TransactionItem {
        date: t.date,
        description: t.description,
        amount: t.amount.amount,
        kind: t.kind,
        category: t.category,
        post_date: t.post_date.filter(|_| detailed),
        memo: mask_text(t.memo).filter(|_| detailed),
        chase_type: mask_text(t.chase_type).filter(|_| detailed),
        balance: t.balance.map(|b| b.amount).filter(|_| detailed),
        check_number: mask_text(t.check_number).filter(|_| detailed),
        file: detailed.then(|| file.to_string()),
    }
}

// ---- chase_exports --------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportsArgs {
    /// Also read the inbox's subfolders. Default: false.
    #[serde(default)]
    pub recursive: bool,
    #[serde(flatten)]
    pub list: ListArgs,
}

#[derive(Serialize)]
struct ExportsFilter {
    recursive: bool,
}

#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct ExportItem {
    /// Relative to the inbox.
    pub file: String,
    /// card-modern, card-legacy or checking. Absent when unreadable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    pub transactions: u64,
    /// Rows that could not be read. Absent when none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    /// The earliest date in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// The latest date in the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// True when the file may be an export and cannot be read to the end.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportsOut {
    /// CSV files found in the inbox, Chase's or not.
    pub csv_files: usize,
    /// Those with a Chase header, or that could not be opened to tell.
    pub chase_exports: usize,
    #[serde(flatten)]
    pub page: Page<ExportItem>,
}

// ---- the toolset ----------------------------------------------------------------

impl<I: Inbox> Toolset for ChaseTools<I> {
    fn server(&self) -> Server {
        Server::new("chse", env!("CARGO_PKG_VERSION"), "Chase", "chase")
            .sign_in_command(SET_INBOX)
            .instructions(
                "There is no connection to Chase here: these read-only tools read CSV files the \
                 user downloaded from chase.com, so they know nothing later than data_through \
                 and cannot change anything. Amounts are USD decimal strings, with charges \
                 positive and payments and refunds negative, the opposite of Chase's own sign. \
                 Each page reads the files again.",
            )
    }

    fn tools(&self) -> Vec<ToolDef> {
        vec![
            ToolDef::read(
                TRANSACTIONS_TOOL,
                "Chase transactions from CSV exports",
                "Lists transactions from the Chase CSV exports the user downloaded, newest first, \
                 with the count and total of everything matching across pages. Use it for what \
                 was spent over a period, on what, or at which merchant. Check data_through, \
                 since nothing later is known, and complete and possible_duplicates before \
                 trusting a total. Narrow with since, until, description, category, kind or file.",
            )
            .input::<TransactionsArgs>()
            .output::<TransactionsOut>(),
            ToolDef::read(
                EXPORTS_TOOL,
                "Chase CSV exports available",
                "Lists the Chase CSV exports in the user's inbox folder: each file's name, variant, \
                 row count and date span. Use it to see which accounts and periods are covered, \
                 or why a total looks short. Files without a Chase header are counted and not \
                 read.",
            )
            .input::<ExportsArgs>()
            .output::<ExportsOut>(),
        ]
    }

    async fn call(&self, name: &str, args: Value) -> Result<ToolOutput, ToolError> {
        match name {
            TRANSACTIONS_TOOL => self.transactions(parse_args(args)?),
            EXPORTS_TOOL => self.exports(parse_args(args)?),
            other => Err(ToolError::other(format!("no handler for {other}"))),
        }
    }
}

impl<I: Inbox> ChaseTools<I> {
    fn transactions(&self, args: TransactionsArgs) -> Result<ToolOutput, ToolError> {
        let window = range(
            non_empty(args.since.as_deref()),
            non_empty(args.until.as_deref()),
        )?;
        let description = non_empty(args.description.as_deref());
        let category = non_empty(args.category.as_deref());
        let kind = non_empty(args.kind.as_deref());
        let file = non_empty(args.file.as_deref());
        let page = PageRequest::new(
            &args.list,
            &TransactionsFilter {
                since: window.since.as_deref(),
                until: window.until.as_deref(),
                description,
                category,
                kind,
                file,
                recursive: args.recursive,
            },
        )?;
        let detailed = args.list.format().is_detailed();

        let reading = read_inbox(&self.inbox.dir()?, args.recursive)?;
        if reading.exports.is_empty() {
            return Err(ToolError::not_found(format!(
                "the inbox folder holds no Chase CSV export ({} other CSV files)",
                reading.csv_files
            ))
            .with_fix(DOWNLOAD_ONE));
        }
        if let Some(wanted) = file {
            if !reading.exports.iter().any(|e| e.name == wanted) {
                let names: Vec<&str> = reading.exports.iter().map(|e| e.name.as_str()).collect();
                return Err(ToolError::not_found(format!(
                    "no export has that name; the exports are: {}",
                    names.join(", ")
                ))
                .with_fix("Call again with one of those names"));
            }
        }

        // Which of the exports being read hold each row, before any other filter, so a
        // duplicate is one whichever row a filter happens to keep, and a row in an export
        // `file` left out adds nothing to a total.
        let selected = |e: &Export| file.is_none_or(|f| f == e.name);
        let mut holders: HashMap<RowKey, HashSet<usize>> = HashMap::new();
        for (i, export) in reading
            .exports
            .iter()
            .enumerate()
            .filter(|(_, e)| selected(e))
        {
            for t in export.parsed.iter().flat_map(|p| &p.transactions) {
                holders.entry(row_key(t)).or_default().insert(i);
            }
        }

        let (mut data_from, mut data_through): (Option<String>, Option<String>) = (None, None);
        let (mut skipped, mut unreadable) = (0, Vec::new());
        let mut rows: Vec<(Transaction, &str)> = Vec::new();
        for export in &reading.exports {
            let Some(parsed) = &export.parsed else {
                unreadable.push(export.name.as_str());
                continue;
            };
            skipped += parsed.errors.len();
            for t in &parsed.transactions {
                if data_from.as_deref().is_none_or(|d| t.date.as_str() < d) {
                    data_from = Some(t.date.clone());
                }
                if data_through.as_deref().is_none_or(|d| t.date.as_str() > d) {
                    data_through = Some(t.date.clone());
                }
            }
            if selected(export) {
                for t in &parsed.transactions {
                    rows.push((t.clone(), export.name.as_str()));
                }
            }
        }

        let contains = |hay: &str, needle: &str| {
            hay.to_ascii_lowercase()
                .contains(&needle.to_ascii_lowercase())
        };
        let mut repeated = 0;
        let mut total_cents: i64 = 0;
        let mut matched: Vec<TransactionItem> = Vec::new();
        rows.sort_by(|a, b| b.0.date.cmp(&a.0.date));
        for (mut t, name) in rows {
            let key = row_key(&t);
            // Text is masked before a filter sees it, so a filter cannot be used to
            // test digits a result would not show.
            t.description = mask_digit_runs(&t.description);
            t.category = mask_text(t.category);
            let keep = window.since.as_deref().is_none_or(|s| t.date.as_str() >= s)
                && window.until.as_deref().is_none_or(|u| t.date.as_str() <= u)
                && description.is_none_or(|d| contains(&t.description, d))
                && category.is_none_or(|c| {
                    t.category
                        .as_deref()
                        .is_some_and(|tc| tc.eq_ignore_ascii_case(c))
                })
                && kind.is_none_or(|k| t.kind.eq_ignore_ascii_case(k));
            if !keep {
                continue;
            }
            total_cents += value::parse_amount_cents(&t.amount.amount).ok_or_else(|| {
                ToolError::other(format!("could not add the amount `{}`", t.amount.amount))
            })?;
            if holders.get(&key).is_some_and(|h| h.len() > 1) {
                repeated += 1;
            }
            matched.push(transaction_item(t, name, detailed));
        }

        let mut note = Vec::new();
        if skipped > 0 {
            note.push(format!(
                "{skipped} rows could not be read and are missing; chase_exports says where."
            ));
        }
        if !unreadable.is_empty() {
            note.push(format!(
                "These exports could not be read and are missing: {}.",
                unreadable.join(", ")
            ));
        }
        if repeated > 0 {
            note.push(format!(
                "{repeated} rows here are identical to one in another export. They may be one \
                 transaction downloaded twice, which total_amount counts twice; narrow with file \
                 to read one export."
            ));
        }
        ToolOutput::new(&TransactionsOut {
            matched: matched.len(),
            total_amount: Money::from_cents(total_cents).amount,
            data_from,
            data_through,
            complete: skipped == 0 && unreadable.is_empty(),
            possible_duplicates: (repeated > 0).then_some(repeated),
            note: (!note.is_empty()).then(|| note.join(" ")),
            page: page.page(matched, "since/until, description, category, kind or file")?,
        })
    }

    fn exports(&self, args: ExportsArgs) -> Result<ToolOutput, ToolError> {
        let page = PageRequest::new(
            &args.list,
            &ExportsFilter {
                recursive: args.recursive,
            },
        )?;
        let reading = read_inbox(&self.inbox.dir()?, args.recursive)?;
        let chase_exports = reading.exports.len();
        let items = reading
            .exports
            .into_iter()
            .map(|export| match export.parsed {
                Some(parsed) => {
                    let report = parsed.report(&export.name);
                    ExportItem {
                        file: export.name,
                        variant: Some(report.variant),
                        transactions: report.transactions,
                        skipped: report.skipped,
                        since: report.since,
                        until: report.until,
                        unreadable: None,
                    }
                }
                None => ExportItem {
                    file: export.name,
                    unreadable: Some(true),
                    ..ExportItem::default()
                },
            })
            .collect();
        ToolOutput::new(&ExportsOut {
            csv_files: reading.csv_files,
            chase_exports,
            page: page.page(items, "recursive")?,
        })
    }
}

#[cfg(test)]
mod tests;
