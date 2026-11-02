//! Output shapes (SPEC v1 §1.4): schema-tagged, snake_case, ISO dates, money
//! as a string decimal, optional fields omitted rather than nulled.

use pk_cli_core::{output, Money, Paged};
use serde::{Deserialize, Serialize};

/// One imported transaction, emitted under the family `transaction-list/v1`
/// envelope.
///
/// **Shape-compatible with `pk_cli_utility::Transaction` by design.** Its four
/// fields — `date`, `amount`, `description`, `kind` — lead this struct in the
/// same order and carry the same meanings, so a consumer that already reads
/// `transaction-list/v1` from a utility CLI reads Chase output with no
/// changes. The remaining fields are Chase-shaped extras it can ignore. We
/// don't depend on `pk-cli-utility` for the type itself: a credit-card export
/// is not a utility portal, `Category`/`Memo`/`Balance` have no home in that
/// DTO, and PROFILES.md's bar for minting a card profile (≥2 CLIs, a consumer
/// paying for the variance today) isn't met by one CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    /// Transaction date, ISO `YYYY-MM-DD`. Chase's `Transaction Date` /
    /// `Trans Date`, falling back to the posting date when only that is given.
    pub date: String,
    /// **Spend-positive**: charges positive, payments and refunds negative —
    /// the family convention, which is the *opposite* of Chase's raw sign.
    /// See `normalize` in `parser.rs`.
    pub amount: Money,
    pub description: String,
    /// Normalized: `charge` | `payment` | `refund` | `fee` | `adjustment` for
    /// cards; `debit` | `credit` | `check` | `deposit` for deposit accounts.
    pub kind: String,

    /// Posting date, when the export distinguishes it from the transaction date.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_date: Option<String>,
    /// Chase's own spend category (`Health & Wellness`, `Home`, …). Verbatim,
    /// not a closed set — it is the single most useful column for deduction
    /// scanning, so it is never rewritten.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// Chase's raw `Type` cell (`Sale`, `Payment`, `ACH_DEBIT`, …), kept for
    /// auditability alongside the normalized `kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chase_type: Option<String>,
    /// Running balance — deposit-account exports only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_number: Option<String>,
    /// Provenance: which export this row came from. Matters once `import scan`
    /// merges several files into one list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

/// Columns shown in text mode. JSON mode always carries the full DTO; a
/// 10-column pipe table would be unreadable in a terminal.
const TRANSACTION_COLUMNS: [&str; 5] = ["date", "description", "amount", "kind", "category"];

/// Emit a page of transactions per the output contract.
pub fn emit_transactions(page: &Paged<Transaction>, json_mode: bool) {
    if json_mode {
        page.emit(true);
        return;
    }
    let items = serde_json::to_value(&page.items).unwrap_or_default();
    let rows = items.as_array().cloned().unwrap_or_default();
    output::table(&output::table_view(&rows, &TRANSACTION_COLUMNS));
}

/// What one CSV file turned out to be — the record `import scan` lists and
/// `import inspect` emits on its own (`chase-csv-file/v1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvFileReport {
    pub path: String,
    /// `card-modern` | `card-legacy` | `checking`.
    pub variant: String,
    /// Rows successfully parsed.
    pub transactions: u64,
    /// Rows skipped as unparseable. Omitted when none were.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skipped: Option<u64>,
    /// Earliest / latest transaction date in the file, ISO.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Per-row parse failures, in file order. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

const REPORT_COLUMNS: [&str; 6] = [
    "path",
    "variant",
    "transactions",
    "skipped",
    "since",
    "until",
];

/// Emit a scan result (`chase-csv-file-list/v1`).
pub fn emit_reports(page: &Paged<CsvFileReport>, json_mode: bool) {
    if json_mode {
        page.emit(true);
        return;
    }
    let items = serde_json::to_value(&page.items).unwrap_or_default();
    let mut rows = items.as_array().cloned().unwrap_or_default();
    // `skipped` is omitted from the DTO when zero (§1.4 omit-don't-null), but
    // the text table draws its columns from whichever keys the rows happen to
    // carry — so without a default the column appears, and moves, depending on
    // whether any file had bad rows. Defaulting it here keeps the column set
    // and its position identical run to run. JSON output is untouched.
    for row in &mut rows {
        if let Some(obj) = row.as_object_mut() {
            obj.entry("skipped").or_insert(serde_json::Value::from(0));
        }
    }
    output::table(&output::table_view(&rows, &REPORT_COLUMNS));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn sample() -> Transaction {
        Transaction {
            date: "2026-07-15".into(),
            amount: Money::from_cents(4210),
            description: "ACME CLINIC".into(),
            kind: "charge".into(),
            post_date: Some("2026-07-16".into()),
            category: Some("Health & Wellness".into()),
            memo: None,
            chase_type: Some("Sale".into()),
            balance: None,
            check_number: None,
            source_file: None,
        }
    }

    #[test]
    fn transaction_leads_with_the_shared_utility_fields() {
        let v = serde_json::to_value(sample()).unwrap();
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
        // Insertion order is part of the contract (SPEC §1.4), and these four
        // are exactly `pk_cli_utility::Transaction`'s fields, in its order.
        assert_eq!(&keys[..4], &["date", "amount", "description", "kind"]);
    }

    #[test]
    fn money_is_a_string_decimal_never_a_float() {
        let v = serde_json::to_value(sample()).unwrap();
        assert_eq!(v["amount"]["amount"], "42.10");
        assert_eq!(v["amount"]["currency"], "USD");
        assert!(v["amount"]["amount"].is_string());
    }

    #[test]
    fn absent_fields_are_omitted_not_nulled() {
        let v = serde_json::to_value(sample()).unwrap();
        for absent in ["memo", "balance", "check_number", "source_file"] {
            assert!(v.get(absent).is_none(), "`{absent}` should be omitted");
        }
        assert_eq!(v["category"], "Health & Wellness");
    }

    #[test]
    fn transactions_page_is_tagged_transaction_list_v1() {
        let page = Paged::new("transaction", vec![sample()]);
        let v: Value = serde_json::to_value(&page).unwrap();
        assert_eq!(v["schema"], "transaction-list/v1");
        assert_eq!(v["items"][0]["date"], "2026-07-15");
    }

    #[test]
    fn scan_page_is_tagged_chase_csv_file_list_v1() {
        let page = Paged::new(
            "chase-csv-file",
            vec![CsvFileReport {
                path: "inbox/activity.csv".into(),
                variant: "card-modern".into(),
                transactions: 3,
                skipped: None,
                since: Some("2026-07-01".into()),
                until: Some("2026-07-15".into()),
                errors: vec![],
            }],
        );
        let v = serde_json::to_value(&page).unwrap();
        assert_eq!(v["schema"], "chase-csv-file-list/v1");
        assert!(v["items"][0].get("skipped").is_none());
        assert!(v["items"][0].get("errors").is_none());
    }
}
