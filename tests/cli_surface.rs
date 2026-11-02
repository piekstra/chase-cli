//! Offline surface tests: the command tree, flags, exit codes, and the JSON
//! output contract (SPEC v1 §1.4/§1.5).
//!
//! No network, no keychain, no real account. `XDG_CONFIG_HOME` is redirected
//! at a scratch directory so nothing here can read or clobber the developer's
//! own `chse` config.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A scratch config home, empty and unwritten by default.
fn config_home() -> PathBuf {
    let dir = std::env::temp_dir().join("chse-surface-tests/config");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn chse() -> Command {
    let mut cmd = Command::cargo_bin("chse").unwrap();
    cmd.env("XDG_CONFIG_HOME", config_home());
    cmd
}

/// Run and parse stdout as exactly one JSON document.
fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    serde_json::from_str(&stdout).expect("stdout is exactly one JSON document")
}

// ---------------------------------------------------------------------------
// Standard surface
// ---------------------------------------------------------------------------

#[test]
fn help_lists_the_standard_surface() {
    chse()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("import"))
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("self-update"))
        .stdout(predicate::str::contains("completions"))
        .stdout(predicate::str::contains("info"));
}

/// Every subcommand's `--help` must render. This is what catches a subcommand
/// flag colliding with a global like `-q` or `-v`, which otherwise only shows
/// up at runtime.
#[test]
fn every_subcommand_help_renders() {
    for args in [
        vec!["import", "--help"],
        vec!["import", "csv", "--help"],
        vec!["import", "scan", "--help"],
        vec!["import", "inspect", "--help"],
        vec!["auth", "--help"],
        vec!["auth", "status", "--help"],
        vec!["config", "--help"],
        vec!["config", "set", "--help"],
        vec!["self-update", "--help"],
        vec!["completions", "--help"],
        vec!["info", "--help"],
    ] {
        chse()
            .args(&args)
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    }
}

#[test]
fn version_prints() {
    chse()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("chse"));
}

#[test]
fn info_emits_cli_info_v1_declaring_no_auth() {
    let v = json_of(chse().arg("info"));
    assert_eq!(v["schema"], "cli-info/v1");
    assert_eq!(v["name"], "chse");
    assert_eq!(v["spec"], "piekstra-cli/1");
    assert_eq!(v["repo"], "https://github.com/piekstra/chase-cli");
    // v0 is offline-only: no credential exists to require.
    assert_eq!(v["auth"]["required"], false);
    assert_eq!(v["auth"]["method"], "none");
    let caps: Vec<&str> = v["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert!(caps.contains(&"import"));
}

#[test]
fn auth_status_reports_credential_free_and_works_logged_out() {
    let v = json_of(chse().args(["--json", "auth", "status"]));
    assert_eq!(v["schema"], "auth-status/v1");
    assert_eq!(v["required"], false);
    assert_eq!(v["method"], "none");
    // Nothing to log into, so every command is usable now.
    assert_eq!(v["authenticated"], true);
}

#[test]
fn completions_render_for_each_shell() {
    for shell in ["bash", "zsh", "fish"] {
        chse()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("chse"));
    }
}

#[test]
fn config_roundtrips_through_an_explicit_path() {
    let path = std::env::temp_dir().join("chse-surface-tests/explicit-config.json");
    let _ = std::fs::remove_file(&path);

    chse()
        .args([
            "--config",
            path.to_str().unwrap(),
            "config",
            "set",
            "inbox",
            "/tmp/inbox",
        ])
        .assert()
        .success();

    let v = json_of(chse().args([
        "--json",
        "--config",
        path.to_str().unwrap(),
        "config",
        "show",
    ]));
    assert_eq!(v["inbox"], "/tmp/inbox");

    chse()
        .args([
            "--config",
            path.to_str().unwrap(),
            "config",
            "unset",
            "inbox",
        ])
        .assert()
        .success();
    let v = json_of(chse().args([
        "--json",
        "--config",
        path.to_str().unwrap(),
        "config",
        "show",
    ]));
    assert!(v.get("inbox").is_none());
}

// ---------------------------------------------------------------------------
// Exit codes and the JSON error DTO
// ---------------------------------------------------------------------------

#[test]
fn unknown_config_key_exits_2_with_a_json_error_dto() {
    let out = chse()
        .args(["--json", "config", "set", "bogus", "x"])
        .assert()
        .code(2);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).expect("error DTO is one JSON document");
    assert_eq!(v["error"]["code"], "usage");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown config key"));
}

#[test]
fn a_missing_file_exits_4_not_found() {
    let out = chse()
        .args(["--json", "import", "csv", "/nonexistent/activity.csv"])
        .assert()
        .code(4);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn a_foreign_csv_exits_5_upstream_naming_the_expected_headers() {
    let out = chse()
        .args(["--json", "import", "csv"])
        .arg(fixture("not-chase.csv"))
        .assert()
        .code(5);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["error"]["code"], "upstream");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("not a recognized Chase CSV export"));
    assert!(
        msg.contains("Transaction Date"),
        "message should say what it wanted"
    );
}

#[test]
fn strict_turns_skipped_rows_into_an_upstream_failure() {
    // Without --strict the file imports, warning on stderr.
    chse()
        .args(["import", "csv"])
        .arg(fixture("malformed-rows.csv"))
        .assert()
        .success()
        .stderr(predicate::str::contains("skipped 3 unparseable row(s)"));

    let out = chse()
        .args(["--json", "import", "csv"])
        .arg(fixture("malformed-rows.csv"))
        .arg("--strict")
        .assert()
        .code(5);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["error"]["code"], "upstream");
}

#[test]
fn an_inverted_date_range_is_a_usage_error_before_any_file_is_read() {
    let out = chse()
        .args(["--json", "import", "csv", "/nonexistent/never-read.csv"])
        .args(["--since", "2026-06-30", "--until", "2026-01-01"])
        .assert()
        .code(2); // usage, not not-found: args are validated first
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["error"]["code"], "usage");
}

#[test]
fn a_non_iso_date_flag_is_rejected() {
    chse()
        .args(["import", "csv"])
        .arg(fixture("card-modern.csv"))
        .args(["--since", "07/01/2026"])
        .assert()
        .code(2);
}

#[test]
fn scan_without_a_directory_or_configured_inbox_is_a_usage_error() {
    let out = chse().args(["--json", "import", "scan"]).assert().code(2);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["error"]["code"], "usage");
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("config set inbox"));
}

#[test]
fn scanning_a_file_instead_of_a_directory_says_which_command_to_use() {
    let out = chse()
        .args(["--json", "import", "scan"])
        .arg(fixture("card-modern.csv"))
        .assert()
        .code(2);
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("import csv"));
}

// ---------------------------------------------------------------------------
// import csv
// ---------------------------------------------------------------------------

#[test]
fn limit_takes_the_most_recent_and_total_reports_the_whole_file() {
    let v = json_of(
        chse()
            .args(["--json", "import", "csv"])
            .arg(fixture("card-modern.csv"))
            .args(["--limit", "3"]),
    );
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(v["total"], 10, "total is the file, not the page");
    assert_eq!(items[0]["date"], "2026-07-18");
    assert_eq!(items[1]["date"], "2026-07-15");
    assert_eq!(items[2]["date"], "2026-07-14");
}

#[test]
fn since_and_until_bound_the_range_inclusively() {
    let v = json_of(
        chse()
            .args(["--json", "import", "csv"])
            .arg(fixture("card-modern.csv"))
            .args(["--since", "2026-07-10", "--until", "2026-07-15"]),
    );
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    for item in items {
        let d = item["date"].as_str().unwrap();
        assert!(
            ("2026-07-10"..="2026-07-15").contains(&d),
            "{d} out of range"
        );
    }
}

#[test]
fn text_mode_writes_a_table_to_stdout_and_notes_to_stderr() {
    chse()
        .args(["import", "csv"])
        .arg(fixture("card-modern.csv"))
        .assert()
        .success()
        // Curated columns, uppercased headers.
        .stdout(predicate::str::contains(
            "DATE | DESCRIPTION | AMOUNT | KIND | CATEGORY",
        ))
        .stdout(predicate::str::contains("ACME FAMILY CLINIC"))
        .stdout(predicate::str::contains("$142.10"))
        // Diagnostics never contaminate stdout.
        .stderr(predicate::str::contains("card-modern"));
}

#[test]
fn quiet_suppresses_the_stderr_note_but_not_the_data() {
    chse()
        .args(["-q", "import", "csv"])
        .arg(fixture("card-modern.csv"))
        .assert()
        .success()
        .stdout(predicate::str::contains("ACME FAMILY CLINIC"))
        .stderr(predicate::str::is_empty());
}

// ---------------------------------------------------------------------------
// import scan
// ---------------------------------------------------------------------------

#[test]
fn scan_finds_chase_exports_by_header_and_ignores_the_rest() {
    let v = json_of(
        chse()
            .args(["--json", "-q", "import", "scan"])
            .arg(fixtures_dir()),
    );
    assert_eq!(v["schema"], "chase-csv-file-list/v1");
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 6, "6 Chase-shaped fixtures at the top level");

    let paths: Vec<&str> = items.iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert!(
        !paths.iter().any(|p| p.contains("not-chase")),
        "a foreign CSV must not be claimed: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.contains('/')),
        "paths are relative to the scanned root: {paths:?}"
    );

    // Every variant is represented and summarized.
    let variants: Vec<&str> = items
        .iter()
        .map(|i| i["variant"].as_str().unwrap())
        .collect();
    for expected in ["card-modern", "card-legacy", "checking"] {
        assert!(variants.contains(&expected), "missing variant {expected}");
    }
    let modern = items
        .iter()
        .find(|i| i["path"] == "card-modern.csv")
        .unwrap();
    assert_eq!(modern["transactions"], 10);
    assert_eq!(modern["since"], "2026-07-01");
    assert_eq!(modern["until"], "2026-07-18");

    // The malformed fixture reports its skips rather than hiding them.
    let bad = items
        .iter()
        .find(|i| i["path"] == "malformed-rows.csv")
        .unwrap();
    assert_eq!(bad["skipped"], 3);
}

#[test]
fn scan_descends_only_with_recursive() {
    let flat = json_of(
        chse()
            .args(["--json", "-q", "import", "scan"])
            .arg(fixtures_dir()),
    );
    let deep = json_of(
        chse()
            .args(["--json", "-q", "import", "scan"])
            .arg(fixtures_dir())
            .arg("--recursive"),
    );
    assert_eq!(flat["items"].as_array().unwrap().len(), 6);
    assert_eq!(deep["items"].as_array().unwrap().len(), 7);

    let paths: Vec<&str> = deep["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap())
        .collect();
    assert!(paths.iter().any(|p| p.contains("nested")));
}

#[test]
fn scan_transactions_merges_every_file_newest_first_with_provenance() {
    let v = json_of(
        chse()
            .args(["--json", "-q", "import", "scan"])
            .arg(fixtures_dir())
            .arg("--transactions"),
    );
    assert_eq!(v["schema"], "transaction-list/v1");
    let items = v["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        26,
        "10 + 3 + 4 + 4 + 4 + 1 across the fixtures"
    );

    // Merged output is sorted newest first regardless of file order.
    let dates: Vec<&str> = items.iter().map(|i| i["date"].as_str().unwrap()).collect();
    let mut sorted = dates.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(dates, sorted);

    // Provenance survives the merge — otherwise a row is untraceable.
    for item in items {
        assert!(item["source_file"].is_string(), "row lost its source_file");
    }
    assert!(items.iter().any(|i| i["source_file"] == "card-legacy.csv"));
}

#[test]
fn scan_reports_a_directory_with_no_chase_exports_as_empty_not_an_error() {
    let empty = std::env::temp_dir().join("chse-surface-tests/empty-inbox");
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::write(empty.join("notes.txt"), "nothing here").unwrap();

    let v = json_of(chse().args(["--json", "-q", "import", "scan"]).arg(&empty));
    assert_eq!(v["schema"], "chase-csv-file-list/v1");
    assert!(v["items"].as_array().unwrap().is_empty());
}

#[test]
fn scan_uses_the_configured_inbox_when_no_directory_is_given() {
    let path = std::env::temp_dir().join("chse-surface-tests/inbox-config.json");
    let _ = std::fs::remove_file(&path);
    chse()
        .args(["--config", path.to_str().unwrap(), "config", "set", "inbox"])
        .arg(fixtures_dir())
        .assert()
        .success();

    let v = json_of(chse().args([
        "--json",
        "-q",
        "--config",
        path.to_str().unwrap(),
        "import",
        "scan",
    ]));
    assert_eq!(v["items"].as_array().unwrap().len(), 6);
}

#[test]
fn a_missing_scan_directory_exits_4() {
    chse()
        .args(["--json", "import", "scan", "/nonexistent/inbox"])
        .assert()
        .code(4);
}
