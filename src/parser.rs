//! Chase CSV → family `Transaction` DTOs.
//!
//! Row-level failures are collected, not fatal: one malformed line in a
//! 400-row export should not cost you the other 399. The count and the
//! messages surface in `chase-csv-file/v1` and on stderr, and `--strict`
//! turns them back into a hard failure for pipelines that want that.

use std::collections::HashMap;
use std::path::Path;

use pk_cli_core::{CliError, Money, RangeArgs};

use crate::dto::{CsvFileReport, Transaction};
use crate::format::{self, Variant};
use crate::value;

/// The outcome of parsing one file.
#[derive(Debug)]
pub struct ParsedFile {
    pub variant: Variant,
    pub transactions: Vec<Transaction>,
    pub errors: Vec<String>,
}

impl ParsedFile {
    /// Summarize as the `chase-csv-file/v1` record.
    pub fn report(&self, path: &str) -> CsvFileReport {
        let mut dates: Vec<&str> = self.transactions.iter().map(|t| t.date.as_str()).collect();
        dates.sort_unstable();
        CsvFileReport {
            path: path.to_string(),
            variant: self.variant.as_str().to_string(),
            transactions: self.transactions.len() as u64,
            skipped: (!self.errors.is_empty()).then_some(self.errors.len() as u64),
            // ISO dates sort lexicographically, so first/last are min/max.
            since: dates.first().map(|d| d.to_string()),
            until: dates.last().map(|d| d.to_string()),
            errors: self.errors.clone(),
        }
    }
}

/// Column lookup by normalized header name.
struct Columns(HashMap<String, usize>);

impl Columns {
    fn new(headers: &[String]) -> Self {
        let mut map = HashMap::new();
        for (i, h) in headers.iter().enumerate() {
            // First occurrence wins; a trailing empty header is ignored.
            let name = format::normalize(h);
            if !name.is_empty() {
                map.entry(name).or_insert(i);
            }
        }
        Columns(map)
    }

    /// The cell for a column, or `None` when the column or the value is absent.
    /// A blank cell reads as absent — Chase leaves `Memo` and `Category` empty
    /// rather than omitting them.
    fn get<'a>(&self, record: &'a csv::StringRecord, name: &str) -> Option<&'a str> {
        let cell = record.get(*self.0.get(name)?)?.trim();
        (!cell.is_empty()).then_some(cell)
    }
}

/// Read and parse a Chase CSV export from disk.
pub fn parse_file(path: &Path) -> Result<ParsedFile, CliError> {
    if !path.exists() {
        return Err(CliError::NotFound(format!(
            "no such file: {}",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| CliError::Other(format!("reading {}: {e}", path.display())))?;
    let source = path.file_name().and_then(|n| n.to_str());
    parse_str(&content, source)
}

/// Parse Chase CSV text. `source` labels each row's `source_file`.
pub fn parse_str(content: &str, source: Option<&str>) -> Result<ParsedFile, CliError> {
    // Chase exports are frequently UTF-8-with-BOM; the csv reader does not
    // strip it, and an unstripped BOM corrupts the first column name.
    let content = content.trim_start_matches('\u{feff}');

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true) // the checking export's trailing comma yields ragged rows
        .trim(csv::Trim::All)
        .from_reader(content.as_bytes());

    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| CliError::Upstream(format!("unreadable CSV header: {e}")))?
        .iter()
        .map(|h| h.to_string())
        .collect();

    let variant = format::detect(&headers).ok_or_else(|| {
        CliError::Upstream(format!(
            "not a recognized Chase CSV export (header: {}). \
             Expected a card export (`Transaction Date,Post Date,Description,…`), \
             a legacy card export (`Type,Trans Date,…`), or a checking export \
             (`Details,Posting Date,…`).",
            headers.join(",")
        ))
    })?;

    let cols = Columns::new(&headers);
    let mut transactions = Vec::new();
    let mut errors = Vec::new();

    for (i, record) in reader.records().enumerate() {
        let row = i + 2; // 1-based, and the header occupies row 1
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                errors.push(format!("row {row}: unreadable ({e})"));
                continue;
            }
        };
        // Trailing blank lines are normal at the end of an export.
        if record.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        match parse_row(&cols, &record, variant, source) {
            Ok(t) => transactions.push(t),
            Err(msg) => errors.push(format!("row {row}: {msg}")),
        }
    }

    Ok(ParsedFile {
        variant,
        transactions,
        errors,
    })
}

fn parse_row(
    cols: &Columns,
    record: &csv::StringRecord,
    variant: Variant,
    source: Option<&str>,
) -> Result<Transaction, String> {
    let description = cols
        .get(record, "description")
        .map(value::clean_description)
        .filter(|d| !d.is_empty())
        .ok_or("missing description")?;

    let raw_amount = cols.get(record, "amount").ok_or("missing amount")?;
    let raw_cents = value::parse_amount_cents(raw_amount)
        .ok_or_else(|| format!("invalid amount `{raw_amount}`"))?;

    let post_date = cols
        .get(record, "post date")
        .or_else(|| cols.get(record, "posting date"))
        .map(|c| value::parse_date(c).ok_or_else(|| format!("invalid posting date `{c}`")))
        .transpose()?;

    // Prefer the transaction date; fall back to the posting date only when the
    // transaction date is genuinely absent. A *present but malformed* date is
    // an error, never a silent substitution — dating a deduction to the wrong
    // day (or the wrong tax year) is exactly the failure we can't ship.
    let txn_date = cols
        .get(record, "transaction date")
        .or_else(|| cols.get(record, "trans date"))
        .map(|c| value::parse_date(c).ok_or_else(|| format!("invalid transaction date `{c}`")))
        .transpose()?;
    let date = txn_date
        .or_else(|| post_date.clone())
        .ok_or("missing date")?;

    let chase_type = cols.get(record, "type").map(str::to_string);

    let (kind, balance, check_number, category, memo) = if variant.is_card() {
        (
            value::card_kind(chase_type.as_deref(), raw_cents),
            None,
            None,
            cols.get(record, "category").map(str::to_string),
            cols.get(record, "memo").map(str::to_string),
        )
    } else {
        let balance = cols
            .get(record, "balance")
            .and_then(value::parse_amount_cents)
            .map(Money::from_cents);
        (
            value::checking_kind(cols.get(record, "details"), raw_cents),
            balance,
            cols.get(record, "check or slip #").map(str::to_string),
            None,
            None,
        )
    };

    Ok(Transaction {
        date,
        // The sign flip. Chase writes purchases negative; the family contract
        // is charges positive / credits negative. Documented in docs/api.md.
        amount: Money::from_cents(-raw_cents),
        description,
        kind,
        post_date,
        category,
        memo,
        chase_type,
        balance,
        check_number,
        source_file: source.map(str::to_string),
    })
}

/// Apply `--since` / `--until` / `--limit`, newest first.
///
/// Sorting before limiting means `--limit N` is "the N most recent", and it
/// gives a stable order when `import scan` merges several files.
pub fn filter_range(mut transactions: Vec<Transaction>, range: &RangeArgs) -> Vec<Transaction> {
    if let Some(since) = &range.since {
        transactions.retain(|t| t.date.as_str() >= since.as_str());
    }
    if let Some(until) = &range.until {
        transactions.retain(|t| t.date.as_str() <= until.as_str());
    }
    transactions.sort_by(|a, b| b.date.cmp(&a.date));
    if let Some(limit) = range.limit {
        transactions.truncate(limit as usize);
    }
    transactions
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODERN: &str = "Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
        07/15/2026,07/16/2026,ACME CLINIC,Health & Wellness,Sale,-42.10,\n\
        07/10/2026,07/10/2026,Payment Thank You - Web,,Payment,500.00,\n";

    #[test]
    fn parses_a_modern_card_export() {
        let p = parse_str(MODERN, Some("activity.csv")).unwrap();
        assert_eq!(p.variant, Variant::CardModern);
        assert_eq!(p.transactions.len(), 2);
        assert!(p.errors.is_empty());

        let charge = &p.transactions[0];
        assert_eq!(charge.date, "2026-07-15");
        assert_eq!(charge.post_date.as_deref(), Some("2026-07-16"));
        assert_eq!(charge.description, "ACME CLINIC");
        assert_eq!(charge.category.as_deref(), Some("Health & Wellness"));
        assert_eq!(charge.kind, "charge");
        assert_eq!(charge.source_file.as_deref(), Some("activity.csv"));
        // Blank Memo is omitted, not an empty string.
        assert!(charge.memo.is_none());
    }

    /// The headline normalization: Chase's negative purchase becomes a
    /// positive charge, and Chase's positive payment becomes negative.
    #[test]
    fn signs_are_normalized_to_spend_positive() {
        let p = parse_str(MODERN, None).unwrap();
        assert_eq!(p.transactions[0].amount.amount, "42.10");
        assert_eq!(p.transactions[0].kind, "charge");
        assert_eq!(p.transactions[1].amount.amount, "-500.00");
        assert_eq!(p.transactions[1].kind, "payment");
    }

    #[test]
    fn parses_a_legacy_card_export() {
        let csv = "Type,Trans Date,Post Date,Description,Amount\n\
            SALE,07/15/2026,07/16/2026,ACME CLINIC,-42.10\n\
            PAYMENT,07/10/2026,07/10/2026,Payment Thank You,500.00\n";
        let p = parse_str(csv, None).unwrap();
        assert_eq!(p.variant, Variant::CardLegacy);
        assert_eq!(p.transactions.len(), 2);
        assert_eq!(p.transactions[0].kind, "charge");
        assert_eq!(p.transactions[0].amount.amount, "42.10");
        assert_eq!(p.transactions[1].kind, "payment");
        // The legacy export has no Category column at all.
        assert!(p.transactions[0].category.is_none());
    }

    #[test]
    fn parses_a_checking_export_with_balance_and_trailing_comma() {
        let csv = "Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #,\n\
            DEBIT,07/15/2026,ACME PLUMBING LLC,-250.00,ACH_DEBIT,1750.25,,\n\
            CREDIT,07/01/2026,PAYROLL DIRECT DEP,2000.00,ACH_CREDIT,2000.25,,\n\
            CHECK,07/05/2026,CHECK #1234,-125.00,CHECK_PAID,1875.25,1234,\n";
        let p = parse_str(csv, None).unwrap();
        assert_eq!(p.variant, Variant::Checking);
        assert_eq!(p.transactions.len(), 3);

        let debit = &p.transactions[0];
        assert_eq!(debit.kind, "debit");
        assert_eq!(debit.amount.amount, "250.00"); // spend-positive
        assert_eq!(debit.balance.as_ref().unwrap().amount, "1750.25"); // not flipped
        assert_eq!(debit.chase_type.as_deref(), Some("ACH_DEBIT"));
        assert_eq!(p.transactions[1].kind, "credit");
        assert_eq!(p.transactions[2].check_number.as_deref(), Some("1234"));
    }

    #[test]
    fn a_bad_row_is_skipped_not_fatal() {
        let csv = "Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
            07/15/2026,07/16/2026,GOOD ROW,Shopping,Sale,-10.00,\n\
            not-a-date,07/16/2026,BAD DATE,Shopping,Sale,-10.00,\n\
            07/17/2026,07/18/2026,,Shopping,Sale,-10.00,\n\
            07/18/2026,07/19/2026,BAD AMOUNT,Shopping,Sale,twelve,\n";
        let p = parse_str(csv, None).unwrap();
        assert_eq!(p.transactions.len(), 1);
        assert_eq!(p.errors.len(), 3);
        assert!(p.errors[0].starts_with("row 3:"), "{:?}", p.errors);
        assert!(p.errors[1].contains("missing description"));
        assert!(p.errors[2].contains("invalid amount"));
    }

    #[test]
    fn blank_trailing_lines_are_not_errors() {
        let csv = format!("{MODERN}\n\n,,,,,,\n");
        let p = parse_str(&csv, None).unwrap();
        assert_eq!(p.transactions.len(), 2);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
    }

    #[test]
    fn quoted_commas_in_descriptions_survive() {
        let csv = "Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
            07/15/2026,07/16/2026,\"ACME CLINIC, PA\",Health & Wellness,Sale,\"-1,234.56\",\n";
        let p = parse_str(csv, None).unwrap();
        assert_eq!(p.transactions[0].description, "ACME CLINIC, PA");
        assert_eq!(p.transactions[0].amount.amount, "1234.56");
    }

    #[test]
    fn a_foreign_csv_is_an_upstream_error() {
        let err = parse_str("Date,Description,Amount\n07/15/2026,X,-1.00\n", None).unwrap_err();
        assert_eq!(err.exit_code(), 5);
        assert!(err
            .to_string()
            .contains("not a recognized Chase CSV export"));
    }

    #[test]
    fn report_summarizes_span_and_skips() {
        let p = parse_str(MODERN, None).unwrap();
        let r = p.report("inbox/activity.csv");
        assert_eq!(r.variant, "card-modern");
        assert_eq!(r.transactions, 2);
        assert_eq!(r.skipped, None);
        assert_eq!(r.since.as_deref(), Some("2026-07-10"));
        assert_eq!(r.until.as_deref(), Some("2026-07-15"));
    }

    #[test]
    fn range_filters_and_sorts_newest_first() {
        let p = parse_str(MODERN, None).unwrap();
        let all = filter_range(p.transactions.clone(), &RangeArgs::default());
        assert_eq!(all[0].date, "2026-07-15");

        let since = RangeArgs {
            since: Some("2026-07-12".into()),
            ..Default::default()
        };
        let filtered = filter_range(p.transactions.clone(), &since);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].date, "2026-07-15");

        let limited = filter_range(
            p.transactions.clone(),
            &RangeArgs {
                limit: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].date, "2026-07-15", "limit takes the most recent");
    }
}
