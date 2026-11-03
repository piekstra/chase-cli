//! `import` — the offline centerpiece: turn Chase's own CSV downloads into
//! family transaction DTOs, and find those downloads in a folder.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use pk_cli_config::ConfigStore;
use pk_cli_core::{output, CliError, CommonArgs, Paged, RangeArgs};

use crate::config::Config;
use crate::dto::{self, CsvFileReport, Transaction};
use crate::parser::{self, ParsedFile};
use crate::scan;

#[derive(Subcommand, Debug)]
pub enum ImportCmd {
    /// Parse one Chase CSV export into transactions (transaction-list/v1).
    Csv(CsvArgs),
    /// Find Chase-shaped CSV exports in a directory (chase-csv-file-list/v1).
    Scan(ScanArgs),
    /// Report what one CSV file is, including per-row parse errors
    /// (chase-csv-file/v1).
    Inspect { file: PathBuf },
}

#[derive(Args, Debug)]
pub struct CsvArgs {
    /// Path to a Chase CSV export.
    pub file: PathBuf,
    #[command(flatten)]
    pub range: RangeArgs,
    /// Fail (exit 5) if any row could not be parsed, instead of skipping it.
    #[arg(long)]
    pub strict: bool,
}

#[derive(Args, Debug)]
pub struct ScanArgs {
    /// Directory to scan. Defaults to the configured `inbox`.
    pub dir: Option<PathBuf>,
    /// Descend into subdirectories.
    #[arg(short, long)]
    pub recursive: bool,
    /// Emit the merged transactions (transaction-list/v1) instead of the
    /// per-file report.
    #[arg(long)]
    pub transactions: bool,
    #[command(flatten)]
    pub range: RangeArgs,
    /// Fail (exit 5) if any row in any file could not be parsed.
    #[arg(long)]
    pub strict: bool,
}

pub fn run(cmd: &ImportCmd, store: &ConfigStore, common: &CommonArgs) -> Result<(), CliError> {
    match cmd {
        ImportCmd::Csv(args) => csv(args, common),
        ImportCmd::Scan(args) => scan_dir(args, store, common),
        ImportCmd::Inspect { file } => inspect(file, common),
    }
}

fn csv(args: &CsvArgs, common: &CommonArgs) -> Result<(), CliError> {
    // Validate arguments before touching the filesystem (SPEC v1: bad input
    // never reaches the provider — here, never reads a file it can't use).
    args.range.validate()?;

    let parsed = parser::parse_file(&args.file)?;
    report_row_errors(
        &parsed,
        &args.file.display().to_string(),
        args.strict,
        common,
    )?;

    let total = parsed.transactions.len() as u64;
    let items = parser::filter_range(parsed.transactions, &args.range);
    note(
        common,
        &format!(
            "{}: {} ({} transactions{})",
            args.file.display(),
            parsed.variant.as_str(),
            total,
            if items.len() as u64 == total {
                String::new()
            } else {
                format!(", {} after filtering", items.len())
            }
        ),
    );

    let mut page = Paged::new("transaction", items);
    page.total = Some(total);
    dto::emit_transactions(&page, common.json);
    Ok(())
}

fn scan_dir(args: &ScanArgs, store: &ConfigStore, common: &CommonArgs) -> Result<(), CliError> {
    args.range.validate()?;
    let dir = resolve_dir(args.dir.as_deref(), store)?;

    let candidates = scan::find_csvs(&dir, args.recursive)?;
    let chase: Vec<PathBuf> = candidates
        .iter()
        .filter(|p| scan::classify(p).is_some())
        .cloned()
        .collect();

    note(
        common,
        &format!(
            "{}: {} CSV file(s), {} Chase-shaped",
            dir.display(),
            candidates.len(),
            chase.len()
        ),
    );

    let mut reports: Vec<CsvFileReport> = Vec::new();
    let mut merged: Vec<Transaction> = Vec::new();
    for path in &chase {
        let parsed = parser::parse_file(path)?;
        let label = display_path(path, &dir);
        report_row_errors(&parsed, &label, args.strict, common)?;
        reports.push(parsed.report(&label));
        merged.extend(parsed.transactions);
    }

    if args.transactions {
        let total = merged.len() as u64;
        let mut page = Paged::new("transaction", parser::filter_range(merged, &args.range));
        page.total = Some(total);
        dto::emit_transactions(&page, common.json);
    } else {
        // `--limit` bounds the file list; the date flags describe transactions
        // and are only meaningful with --transactions, so they don't filter here.
        if let Some(limit) = args.range.limit {
            reports.truncate(limit as usize);
        }
        let page = Paged::new("chase-csv-file", reports);
        dto::emit_reports(&page, common.json);
    }
    Ok(())
}

fn inspect(file: &Path, common: &CommonArgs) -> Result<(), CliError> {
    let parsed = parser::parse_file(file)?;
    let report = parsed.report(&file.display().to_string());
    let payload = serde_json::to_value(&report)
        .map_err(|e| CliError::Other(format!("serializing report: {e}")))?;
    output::emit(common.json, "chase-csv-file", payload, |v| {
        // Render the summary without `errors`, then list them one per line.
        // Left in, `kv` renders the array comma-joined on a single line, which
        // both duplicates the list below and is unreadable past two entries.
        let mut summary = v.clone();
        if let Some(obj) = summary.as_object_mut() {
            obj.remove("errors");
        }
        output::kv(&summary, 0);
        if !parsed.errors.is_empty() {
            println!("errors:");
            for err in &parsed.errors {
                println!("  {err}");
            }
        }
    });
    Ok(())
}

/// Directory argument, else the configured inbox, else a usage error naming
/// both ways to supply it.
fn resolve_dir(arg: Option<&Path>, store: &ConfigStore) -> Result<PathBuf, CliError> {
    if let Some(dir) = arg {
        return Ok(dir.to_path_buf());
    }
    let cfg: Config = store.load()?;
    cfg.inbox.map(PathBuf::from).ok_or_else(|| {
        CliError::Usage(
            "no directory given and no `inbox` configured — pass a DIR or run \
             `chse config set inbox <PATH>`"
                .into(),
        )
    })
}

/// Paths in the report are relative to the scanned root when possible, so the
/// output doesn't leak the full home-directory layout into a shared log.
pub fn display_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Row-level parse failures: loud on stderr, fatal only under `--strict`.
fn report_row_errors(
    parsed: &ParsedFile,
    label: &str,
    strict: bool,
    common: &CommonArgs,
) -> Result<(), CliError> {
    if parsed.errors.is_empty() {
        return Ok(());
    }
    if strict {
        return Err(CliError::Upstream(format!(
            "{label}: {} row(s) could not be parsed (--strict): {}",
            parsed.errors.len(),
            parsed.errors.join("; ")
        )));
    }
    if !common.quiet {
        eprintln!(
            "warning: {label}: skipped {} unparseable row(s)",
            parsed.errors.len()
        );
        if common.verbose {
            for err in &parsed.errors {
                eprintln!("  {err}");
            }
        }
    }
    Ok(())
}

/// A progress note on stderr — never stdout, which carries data only.
fn note(common: &CommonArgs, msg: &str) {
    if !common.quiet {
        eprintln!("{msg}");
    }
}
