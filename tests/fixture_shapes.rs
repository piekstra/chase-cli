//! Contract tests over the synthetic fixtures in `tests/fixtures/`.
//!
//! These drive the real binary and assert on the JSON it emits, so they cover
//! the whole path — detection, parsing, normalization, DTO shape — and fail
//! loudly if a column the table views read ever stops being populated.
//!
//! Everything is offline. Nothing here touches the network or a real account.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture(name: &str) -> PathBuf {
    fixtures().join(name)
}

/// Parse a fixture through the CLI and return the emitted DTO.
fn import(name: &str) -> Value {
    let out = Command::cargo_bin("chse")
        .unwrap()
        .args(["--json", "import", "csv"])
        .arg(fixture(name))
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    serde_json::from_str(&stdout).expect("stdout is exactly one JSON document")
}

fn items(v: &Value) -> Vec<Value> {
    v["items"].as_array().cloned().unwrap_or_default()
}

/// The row whose description contains `needle`.
fn row<'a>(rows: &'a [Value], needle: &str) -> &'a Value {
    rows.iter()
        .find(|r| {
            r["description"]
                .as_str()
                .unwrap_or_default()
                .contains(needle)
        })
        .unwrap_or_else(|| panic!("no row matching `{needle}`"))
}

#[test]
fn card_modern_carries_every_column_the_table_view_reads() {
    let v = import("card-modern.csv");
    assert_eq!(v["schema"], "transaction-list/v1");
    assert_eq!(v["total"], 10);

    let rows = items(&v);
    assert_eq!(rows.len(), 10);
    for r in &rows {
        for field in ["date", "amount", "description", "kind"] {
            assert!(r.get(field).is_some(), "row missing `{field}`: {r}");
        }
        // Money is always a string decimal, never a float (SPEC §1.4).
        assert!(r["amount"]["amount"].is_string(), "amount must be a string");
        assert_eq!(r["amount"]["currency"], "USD");
        // ISO dates everywhere.
        let date = r["date"].as_str().unwrap();
        assert_eq!(date.len(), 10, "date `{date}` is not ISO YYYY-MM-DD");
        assert_eq!(date.matches('-').count(), 2);
    }
    // The columns that make this useful for deduction scanning.
    assert!(rows.iter().any(|r| r["category"] == "Health & Wellness"));
    assert!(rows.iter().any(|r| r.get("memo").is_some()));
    assert!(rows.iter().any(|r| r["post_date"].is_string()));
}

/// The headline normalization: Chase writes purchases negative, the family
/// contract is charges-positive. Getting this backwards would invert every
/// deduction total downstream, so it is pinned per `Type` value.
#[test]
fn signs_are_normalized_to_spend_positive() {
    let rows = items(&import("card-modern.csv"));

    let charge = row(&rows, "ACME FAMILY CLINIC");
    assert_eq!(charge["amount"]["amount"], "142.10");
    assert_eq!(charge["kind"], "charge");

    let payment = row(&rows, "Payment Thank You");
    assert_eq!(payment["amount"]["amount"], "-2500.00");
    assert_eq!(payment["kind"], "payment");

    // A `Return` is a credit in Chase's file (+45.00) and must reduce spend.
    let refund = rows
        .iter()
        .find(|r| r["kind"] == "refund")
        .expect("fixture has a Return row");
    assert_eq!(refund["amount"]["amount"], "-45.00");

    let fee = row(&rows, "ANNUAL MEMBERSHIP FEE");
    assert_eq!(fee["amount"]["amount"], "95.00");
    assert_eq!(fee["kind"], "fee");
}

#[test]
fn quoted_commas_thousands_separators_and_entities_survive() {
    let rows = items(&import("card-modern.csv"));

    // Description is quoted because it contains a comma; amount is quoted
    // because it contains a thousands separator.
    let hvac = row(&rows, "ACME HVAC");
    assert_eq!(hvac["description"], "ACME HVAC SERVICES, INC");
    assert_eq!(hvac["amount"]["amount"], "1250.00");

    // `&amp;` decodes; the merchant name is otherwise left alone.
    let lawn = row(&rows, "LAWN");
    assert_eq!(lawn["description"], "EXAMPLE LAWN & GARDEN CO");
}

#[test]
fn card_export_without_memo_column_still_parses() {
    let v = import("card-modern-no-memo.csv");
    let rows = items(&v);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.get("memo").is_none()));
    assert_eq!(
        row(&rows, "ACME FAMILY CLINIC")["amount"]["amount"],
        "88.00"
    );
}

#[test]
fn legacy_card_export_parses_without_a_category_column() {
    let rows = items(&import("card-legacy.csv"));
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter().all(|r| r.get("category").is_none()),
        "the legacy export has no Category column"
    );
    assert_eq!(row(&rows, "ACME FAMILY CLINIC")["kind"], "charge");
    assert_eq!(row(&rows, "PAYMENT THANK YOU")["kind"], "payment");
    assert_eq!(row(&rows, "ACME HARDWARE")["kind"], "refund");
}

#[test]
fn checking_export_carries_balance_and_check_number() {
    let rows = items(&import("checking.csv"));
    assert_eq!(rows.len(), 4);

    let debit = row(&rows, "ACME PLUMBING");
    assert_eq!(debit["kind"], "debit");
    assert_eq!(debit["amount"]["amount"], "385.00", "spend-positive");
    // The running balance is an account state, not a spend — never flipped.
    assert_eq!(debit["balance"]["amount"], "4395.25");
    assert_eq!(debit["chase_type"], "ACH_DEBIT");

    let deposit = row(&rows, "PAYROLL");
    assert_eq!(deposit["kind"], "credit");
    assert_eq!(deposit["amount"]["amount"], "-5000.00");

    let check = row(&rows, "CHECK #");
    assert_eq!(check["kind"], "check");
    assert_eq!(check["check_number"], "1234");
}

/// A card export that has been through a spreadsheet: BOM, four date
/// spellings, and three ways of writing a negative amount.
#[test]
fn spreadsheet_roundtrip_quirks_all_parse() {
    let rows = items(&import("card-spreadsheet-roundtrip.csv"));
    assert_eq!(rows.len(), 4, "every quirky row must survive");

    let clinic = row(&rows, "ACME FAMILY CLINIC");
    assert_eq!(clinic["date"], "2026-05-20", "ISO date");
    assert_eq!(
        clinic["amount"]["amount"], "142.10",
        "($142.10) is negative"
    );

    let plumbing = row(&rows, "ACME PLUMBING");
    assert_eq!(plumbing["date"], "2026-05-18", "M/D/YY date");
    assert_eq!(plumbing["amount"]["amount"], "1385.00", "-$1,385.00");

    let pharmacy = row(&rows, "EXAMPLE PHARMACY");
    assert_eq!(pharmacy["date"], "2026-05-15", "long-form date");
    assert_eq!(pharmacy["amount"]["amount"], "24.99", "trailing minus");

    let payment = row(&rows, "Payment Thank You");
    assert_eq!(payment["amount"]["amount"], "-500.00");
}

#[test]
fn malformed_rows_are_skipped_and_reported_with_row_numbers() {
    let v = import("malformed-rows.csv");
    assert_eq!(items(&v).len(), 1, "only the good row is imported");

    let out = Command::cargo_bin("chse")
        .unwrap()
        .args(["--json", "import", "inspect"])
        .arg(fixture("malformed-rows.csv"))
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(report["schema"], "chase-csv-file/v1");
    assert_eq!(report["variant"], "card-modern");
    assert_eq!(report["transactions"], 1);
    assert_eq!(report["skipped"], 3);
    let errors = report["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 3);
    assert!(errors[0].as_str().unwrap().starts_with("row 3:"));
    assert!(errors
        .iter()
        .any(|e| e.as_str().unwrap().contains("invalid transaction date")));
    assert!(errors
        .iter()
        .any(|e| e.as_str().unwrap().contains("missing description")));
    assert!(errors
        .iter()
        .any(|e| e.as_str().unwrap().contains("invalid amount")));
}

#[test]
fn a_clean_file_reports_no_skips_or_errors() {
    let out = Command::cargo_bin("chse")
        .unwrap()
        .args(["--json", "import", "inspect"])
        .arg(fixture("card-modern.csv"))
        .assert()
        .success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["transactions"], 10);
    // Omitted, not zero/null (SPEC §1.4 omit-don't-null).
    assert!(report.get("skipped").is_none());
    assert!(report.get("errors").is_none());
    assert_eq!(report["since"], "2026-07-01");
    assert_eq!(report["until"], "2026-07-18");
}

// ---------------------------------------------------------------------------
// Fixture data policy (see tests/fixtures/README.md)
// ---------------------------------------------------------------------------

/// Every fixture CSV on disk, including nested ones.
fn all_fixture_csvs() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv")) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(&fixtures(), &mut out);
    out.sort();
    assert!(out.len() >= 8, "expected the full fixture set, got {out:?}");
    out
}

/// Stated positively: an identity-bearing field must match a known dummy
/// shape. A denylist of real merchant names would publish exactly what it
/// exists to keep out.
#[test]
fn fixture_descriptions_are_recognizably_synthetic() {
    const SYNTHETIC: [&str; 2] = ["ACME", "EXAMPLE"];
    const BANK_GENERATED: [&str; 5] = [
        "PAYMENT THANK YOU",
        "ANNUAL MEMBERSHIP FEE",
        "INTEREST CHARGE",
        "CHECK #",
        "PAYROLL DIRECT DEP",
    ];

    for path in all_fixture_csvs() {
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .trim(csv::Trim::All)
            .from_path(&path)
            .unwrap();
        let headers = reader.headers().unwrap().clone();
        let col = headers
            .iter()
            .position(|h| {
                h.trim_start_matches('\u{feff}')
                    .eq_ignore_ascii_case("description")
            })
            .unwrap_or_else(|| panic!("{path:?} has no Description column"));

        for record in reader.records().flatten() {
            let Some(desc) = record.get(col) else {
                continue;
            };
            let desc = desc.trim();
            if desc.is_empty() {
                continue;
            }
            let upper = desc.to_uppercase();
            let ok = SYNTHETIC.iter().any(|m| upper.contains(m))
                || BANK_GENERATED.iter().any(|m| upper.contains(m));
            assert!(
                ok,
                "{path:?}: description `{desc}` carries no synthetic marker — \
                 fixtures must be invented, never captured"
            );
        }
    }
}

#[test]
fn fixtures_contain_no_account_shaped_numbers_or_addresses() {
    for path in all_fixture_csvs() {
        let body = std::fs::read_to_string(&path).unwrap();

        let mut run = 0usize;
        for ch in body.chars() {
            run = if ch.is_ascii_digit() { run + 1 } else { 0 };
            assert!(
                run < 8,
                "{path:?} contains a run of {run}+ digits — that is the shape of \
                 an account, card, or routing number"
            );
        }
        assert!(
            !body.contains('@'),
            "{path:?} contains an email-shaped value"
        );
    }
}

#[test]
fn fixture_policy_is_documented() {
    let readme = std::fs::read_to_string(fixtures().join("README.md")).unwrap();
    assert!(readme.to_lowercase().contains("synthetic"));
    assert!(
        readme.contains("ACME") && readme.contains("EXAMPLE"),
        "the README must record the markers the policy test enforces"
    );
}
