//! The tools over the synthetic fixtures in `tests/fixtures/`, and over scratch
//! folders where a test needs a file the fixture policy would not allow.

use std::path::Path;

use pk_cli_mcp::conformance::{assert_conforms, assert_fits, call, definitions_size};
use pk_cli_mcp::{tools_list_json, CallToolResult};
use serde_json::{json, Value};

use super::*;

struct At(PathBuf);

impl Inbox for At {
    fn dir(&self) -> Result<PathBuf, ToolError> {
        Ok(self.0.clone())
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn tools() -> ChaseTools<At> {
    ChaseTools::new(At(fixtures()))
}

/// A scratch inbox of its own per test, holding these files.
fn inbox(test: &str, files: &[(&str, &str)]) -> ChaseTools<At> {
    let dir = std::env::temp_dir().join(format!("chse-mcp-tests-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
    ChaseTools::new(At(dir))
}

const CARD_HEADER: &str = "Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n";

fn text(result: &CallToolResult) -> String {
    result.text().to_string()
}

fn error<I: Inbox>(tools: &ChaseTools<I>, tool: &str, args: Value) -> String {
    let result = call(tools, tool, args);
    assert!(result.is_error(), "{}", text(&result));
    text(&result)
}

fn ok<I: Inbox>(tools: &ChaseTools<I>, tool: &str, args: Value) -> Value {
    let result = call(tools, tool, args);
    assert_fits(tools, tool, &result).clone()
}

// ---- the profile -------------------------------------------------------------

#[test]
fn the_tools_conform_to_the_profile() {
    assert_conforms(&tools());
    assert!(definitions_size(&tools()) < 8_000);
}

#[test]
fn listing_the_tools_reads_nothing() {
    struct Refuses;
    impl Inbox for Refuses {
        fn dir(&self) -> Result<PathBuf, ToolError> {
            panic!("listing the tools must not read the config");
        }
    }
    let tools = ChaseTools::new(Refuses);
    let names: Vec<String> = tools.tools().into_iter().map(|t| t.name).collect();
    assert_eq!(names, [TRANSACTIONS_TOOL, EXPORTS_TOOL]);
    let _ = tools_list_json(&tools);
}

#[test]
fn every_definition_is_a_read_and_no_input_takes_a_path() {
    let list = tools_list_json(&tools());
    for tool in list["tools"].as_array().unwrap() {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
        assert_eq!(tool["annotations"]["destructiveHint"], false, "{tool}");
        for key in tool["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
        {
            assert!(
                !["path", "dir", "directory", "inbox"].contains(&key.as_str()),
                "{key}"
            );
        }
    }
}

/// `file` is compared with the names found, never opened: a path outside the inbox
/// is simply not one of them, and the error does not read it.
#[test]
fn a_call_cannot_reach_a_file_outside_the_inbox() {
    let outside = inbox(
        "outside-target",
        &[(
            "secret.csv",
            &format!("{CARD_HEADER}07/15/2026,07/16/2026,ACME OUTSIDE,Home,Sale,-1.00,\n"),
        )],
    );
    let path = outside.inbox.0.join("secret.csv");
    for file in [
        path.display().to_string(),
        "../secret.csv".to_string(),
        "/etc/passwd".to_string(),
    ] {
        let says = error(&tools(), TRANSACTIONS_TOOL, json!({ "file": file }));
        assert!(says.starts_with("not_found:"), "{says}");
        assert!(!says.contains("ACME OUTSIDE"), "{says}");
    }
    // The schema has no way to name a folder either, and an unknown argument is
    // refused rather than ignored.
    let says = error(
        &tools(),
        TRANSACTIONS_TOOL,
        json!({ "inbox": outside.inbox.0.display().to_string() }),
    );
    assert!(says.starts_with("usage:"), "{says}");
}

// ---- chase_transactions ----------------------------------------------------------

#[test]
fn transactions_come_newest_first_spend_positive() {
    let out = ok(
        &tools(),
        TRANSACTIONS_TOOL,
        json!({"file": "card-modern.csv"}),
    );
    let items = out["items"].as_array().unwrap();
    assert_eq!(items[0]["date"], "2026-07-18");
    assert_eq!(items[0]["description"], "EXAMPLE PHARMACY #123");
    assert_eq!(items[0]["amount"], "24.99", "Chase writes this -24.99");
    assert_eq!(items[0]["kind"], "charge");
    assert_eq!(items[0]["category"], "Health & Wellness");
    assert!(items[0].get("memo").is_none(), "concise");
    assert!(items[0].get("file").is_none(), "concise");
    let dates: Vec<&str> = items.iter().map(|i| i["date"].as_str().unwrap()).collect();
    let mut sorted = dates.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(dates, sorted);
}

#[test]
fn detailed_adds_the_memo_type_and_file() {
    let out = ok(
        &tools(),
        TRANSACTIONS_TOOL,
        json!({"file": "card-modern.csv", "response_format": "detailed", "description": "hvac"}),
    );
    let item = &out["items"][0];
    assert_eq!(item["memo"], "Rental unit AC repair");
    assert_eq!(item["chase_type"], "Sale");
    assert_eq!(item["post_date"], "2026-07-15");
    assert_eq!(item["file"], "card-modern.csv");
}

#[test]
fn the_count_and_total_cover_every_page() {
    let tools = inbox(
        "totals",
        &[(
            "a.csv",
            &format!(
                "{CARD_HEADER}07/03/2026,07/03/2026,ACME A,Home,Sale,-10.10,\n\
                 07/02/2026,07/02/2026,ACME B,Home,Sale,-20.20,\n\
                 07/01/2026,07/01/2026,ACME C,Home,Return,5.05,\n"
            ),
        )],
    );
    let first = ok(&tools, TRANSACTIONS_TOOL, json!({"limit": 1}));
    assert_eq!(first["items"].as_array().unwrap().len(), 1);
    assert_eq!(first["matched"], 3);
    assert_eq!(first["total_amount"], "25.25");
    assert_eq!(first["data_from"], "2026-07-01");
    assert_eq!(first["data_through"], "2026-07-03");
    assert_eq!(first["complete"], true);

    let cursor = first["next_cursor"].as_str().unwrap();
    let next = ok(
        &tools,
        TRANSACTIONS_TOOL,
        json!({"limit": 1, "cursor": cursor}),
    );
    assert_ne!(first["items"][0], next["items"][0]);
    assert_eq!(next["total_amount"], "25.25");

    let says = error(
        &tools,
        TRANSACTIONS_TOOL,
        json!({"limit": 1, "cursor": cursor, "kind": "charge"}),
    );
    assert!(says.starts_with("usage:"), "{says}");
}

#[test]
fn the_filters_narrow_and_data_through_does_not_move() {
    let tools = inbox(
        "filters",
        &[(
            "a.csv",
            &format!(
                "{CARD_HEADER}07/03/2026,07/03/2026,ACME GROCER,Groceries,Sale,-10.00,\n\
                 07/02/2026,07/02/2026,EXAMPLE CLINIC,Health & Wellness,Sale,-20.00,\n\
                 07/01/2026,07/01/2026,Payment Thank You,,Payment,30.00,\n"
            ),
        )],
    );
    for (args, want) in [
        (json!({"category": "groceries"}), "ACME GROCER"),
        (json!({"description": "clinic"}), "EXAMPLE CLINIC"),
        (json!({"kind": "PAYMENT"}), "Payment Thank You"),
        (json!({"since": "2026-07-03"}), "ACME GROCER"),
        (json!({"until": "2026-07-01"}), "Payment Thank You"),
        (
            json!({"since": "2026-07-02", "until": "2026-07-02"}),
            "EXAMPLE CLINIC",
        ),
    ] {
        let out = ok(&tools, TRANSACTIONS_TOOL, args.clone());
        assert_eq!(out["matched"], 1, "{args}");
        assert_eq!(out["items"][0]["description"], want, "{args}");
        assert_eq!(out["data_through"], "2026-07-03", "{args}");
    }
}

#[test]
fn a_bad_range_is_a_usage_error() {
    for args in [
        json!({"since": "2026-13-01"}),
        json!({"until": "yesterday"}),
        json!({"since": "2026-02-01", "until": "2026-01-01"}),
    ] {
        let says = error(&tools(), TRANSACTIONS_TOOL, args);
        assert!(says.starts_with("usage:"), "{says}");
    }
}

#[test]
fn skipped_rows_make_a_list_incomplete_and_say_how_many() {
    let out = ok(
        &tools(),
        TRANSACTIONS_TOOL,
        json!({"file": "malformed-rows.csv"}),
    );
    assert_eq!(out["complete"], false);
    assert_eq!(out["matched"], 1);
    assert!(out["note"].as_str().unwrap().contains("3 rows"), "{out}");
}

/// A row's own text in a parse error can quote a cell, so errors are counted here.
#[test]
fn a_skipped_rows_content_is_not_repeated() {
    let out = ok(
        &tools(),
        TRANSACTIONS_TOOL,
        json!({"file": "malformed-rows.csv"}),
    );
    assert!(!out.to_string().contains("not-a-date"), "{out}");
    let out = ok(&tools(), EXPORTS_TOOL, json!({}));
    assert!(!out.to_string().contains("not-a-date"), "{out}");
}

#[test]
fn a_row_in_two_exports_is_flagged_and_two_in_one_are_not() {
    let row = "07/02/2026,07/02/2026,ACME COFFEE,Food,Sale,-4.00,\n";
    let tools = inbox(
        "duplicates",
        &[
            ("june.csv", &format!("{CARD_HEADER}{row}")),
            ("july.csv", &format!("{CARD_HEADER}{row}")),
        ],
    );
    let out = ok(&tools, TRANSACTIONS_TOOL, json!({}));
    assert_eq!(out["matched"], 2);
    assert_eq!(out["possible_duplicates"], 2);
    assert_eq!(out["complete"], true);
    assert!(
        out["note"].as_str().unwrap().contains("downloaded twice"),
        "{out}"
    );

    let out = ok(&tools, TRANSACTIONS_TOOL, json!({"file": "june.csv"}));
    assert_eq!(out["matched"], 1);
    assert_eq!(out["total_amount"], "4.00");

    let tools = inbox(
        "same-day-twice",
        &[("a.csv", &format!("{CARD_HEADER}{row}{row}"))],
    );
    let out = ok(&tools, TRANSACTIONS_TOOL, json!({}));
    assert_eq!(out["matched"], 2);
    assert!(out.get("possible_duplicates").is_none(), "{out}");
}

/// A checking export's descriptions can carry an account number. Dummy digits.
#[test]
fn a_long_digit_run_is_masked_and_a_filter_cannot_find_it() {
    let tools = inbox(
        "masking",
        &[(
            "statement 000000001234.csv",
            &format!(
                "{CARD_HEADER}07/02/2026,07/02/2026,ACME ACH ID 000000005678,Home,Sale,-1.00,\
                 ref 000000009012\n"
            ),
        )],
    );
    let out = ok(
        &tools,
        TRANSACTIONS_TOOL,
        json!({"response_format": "detailed"}),
    );
    let item = &out["items"][0];
    assert_eq!(item["description"], "ACME ACH ID ****5678");
    assert_eq!(item["memo"], "ref ****9012");
    assert_eq!(item["file"], "statement ****1234.csv");
    assert!(!out.to_string().contains("00000000"), "{out}");

    let out = ok(
        &tools,
        TRANSACTIONS_TOOL,
        json!({"description": "000000005678"}),
    );
    assert_eq!(out["matched"], 0);

    let out = ok(&tools, EXPORTS_TOOL, json!({}));
    assert_eq!(out["items"][0]["file"], "statement ****1234.csv");
}

#[test]
fn a_short_digit_run_is_left_alone() {
    assert_eq!(mask_digit_runs("CHECK #1234"), "CHECK #1234");
    assert_eq!(mask_digit_runs("1234567"), "1234567");
    assert_eq!(mask_digit_runs("12345678"), "****5678");
    assert_eq!(mask_digit_runs("a12345678b1"), "a****5678b1");
}

// ---- an inbox with nothing to read -----------------------------------------------

#[test]
fn an_inbox_with_no_export_is_an_error_and_not_no_transactions() {
    let tools = inbox("empty", &[("other.csv", "Date,Debit\n07/15/2026,1.00\n")]);
    let says = error(&tools, TRANSACTIONS_TOOL, json!({}));
    assert!(says.starts_with("not_found:"), "{says}");
    assert!(says.contains("chase.com"), "{says}");

    let out = ok(&tools, EXPORTS_TOOL, json!({}));
    assert_eq!(out["csv_files"], 1);
    assert_eq!(out["chase_exports"], 0);
}

#[test]
fn a_missing_inbox_is_an_error_that_does_not_print_its_path() {
    let tools = ChaseTools::new(At(PathBuf::from("/nonexistent/chse-inbox")));
    for tool in [TRANSACTIONS_TOOL, EXPORTS_TOOL] {
        let says = error(&tools, tool, json!({}));
        assert!(says.starts_with("not_found:"), "{says}");
        assert!(says.contains("chse config set inbox"), "{says}");
        assert!(!says.contains("nonexistent"), "{says}");
    }
}

#[test]
fn an_inbox_that_is_a_file_is_an_error_that_does_not_print_its_path() {
    let tools = ChaseTools::new(At(fixtures().join("card-modern.csv")));
    let says = error(&tools, TRANSACTIONS_TOOL, json!({}));
    assert!(says.starts_with("not_found:"), "{says}");
    assert!(!says.contains("card-modern"), "{says}");
}

/// A Chase header over bytes that are not UTF-8: the rest of the inbox still answers,
/// and the answer says what is missing.
#[test]
fn an_export_that_cannot_be_read_is_named_and_the_rest_still_answer() {
    let tools = inbox(
        "unreadable",
        &[(
            "good.csv",
            &format!("{CARD_HEADER}07/02/2026,07/02/2026,ACME,Home,Sale,-1.00,\n"),
        )],
    );
    let bad = tools.inbox.0.join("bad.csv");
    let mut bytes = CARD_HEADER.as_bytes().to_vec();
    bytes.extend_from_slice(b"07/02/2026,07/02/2026,ACME \xff,Home,Sale,-1.00,\n");
    std::fs::write(bad, bytes).unwrap();

    let out = ok(&tools, TRANSACTIONS_TOOL, json!({}));
    assert_eq!(out["complete"], false);
    assert_eq!(out["matched"], 1);
    assert!(out["note"].as_str().unwrap().contains("bad.csv"), "{out}");

    let out = ok(&tools, EXPORTS_TOOL, json!({}));
    let bad = out["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["file"] == "bad.csv")
        .unwrap();
    assert_eq!(bad["unreadable"], true);
}

// ---- chase_exports ---------------------------------------------------------------

#[test]
fn exports_name_each_file_its_type_and_span() {
    let out = ok(&tools(), EXPORTS_TOOL, json!({"limit": 200}));
    let items = out["items"].as_array().unwrap();
    let modern = items
        .iter()
        .find(|i| i["file"] == "card-modern.csv")
        .unwrap();
    assert_eq!(modern["type"], "card-modern");
    assert_eq!(modern["until"], "2026-07-18");
    assert!(modern["transactions"].as_u64().unwrap() > 0);

    let malformed = items
        .iter()
        .find(|i| i["file"] == "malformed-rows.csv")
        .unwrap();
    assert_eq!(malformed["skipped"], 3);

    assert!(items.iter().all(|i| i["file"] != "not-chase.csv"), "{out}");
    assert_eq!(out["chase_exports"], items.len());
    assert_eq!(
        out["csv_files"],
        items.len() + 1,
        "not-chase.csv is counted"
    );
}

#[test]
fn recursive_reaches_subfolders_and_the_default_does_not() {
    let flat = ok(&tools(), EXPORTS_TOOL, json!({"limit": 200}));
    assert!(!flat.to_string().contains("card-nested"), "{flat}");
    let deep = ok(
        &tools(),
        EXPORTS_TOOL,
        json!({"limit": 200, "recursive": true}),
    );
    assert!(deep.to_string().contains("card-nested"), "{deep}");
    assert!(
        !deep.to_string().contains(env!("CARGO_MANIFEST_DIR")),
        "{deep}"
    );
}
