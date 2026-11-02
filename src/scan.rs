//! Finding Chase exports in a directory (e.g. the Drive inbox).
//!
//! Classification is by **header signature**, not filename: a Drive inbox is
//! full of `download (3).csv` and `Untitled spreadsheet.csv`, and Chase's own
//! export name varies by account and date range. Reading one line per
//! candidate keeps a scan of a large folder cheap.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use pk_cli_core::CliError;

use crate::format::{self, Variant};

/// Collect `.csv` files under `root`, sorted for deterministic output.
///
/// Hidden entries are skipped: a Drive/Dropbox folder is full of dotfiles and
/// sync metadata, and none of it is a bank export.
pub fn find_csvs(root: &Path, recursive: bool) -> Result<Vec<PathBuf>, CliError> {
    if !root.exists() {
        return Err(CliError::NotFound(format!(
            "no such directory: {}",
            root.display()
        )));
    }
    if !root.is_dir() {
        return Err(CliError::Usage(format!(
            "{} is a file, not a directory — use `chse import csv` for a single export",
            root.display()
        )));
    }
    let mut found = Vec::new();
    collect(root, recursive, &mut found)?;
    found.sort();
    Ok(found)
}

fn collect(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> Result<(), CliError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| CliError::Other(format!("reading {}: {e}", dir.display())))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            if recursive {
                collect(&path, recursive, out)?;
            }
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Classify a file by reading only its header row. `None` means "not a Chase
/// export" — including unreadable/binary files, which we skip rather than fail
/// on, since a scan should survive whatever else is sitting in the folder.
pub fn classify(path: &Path) -> Option<Variant> {
    let file = std::fs::File::open(path).ok()?;
    let mut first_line = String::new();
    // `read_line` on invalid UTF-8 errors out, which is the answer we want.
    BufReader::new(file).read_line(&mut first_line).ok()?;

    let line = first_line.trim_start_matches('\u{feff}');
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(line.as_bytes());
    let headers: Vec<String> = reader.headers().ok()?.iter().map(String::from).collect();
    format::detect(&headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory under the OS temp dir, so tests never touch $HOME
    /// and never read the user's real inbox.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("chse-scan-tests-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, body).unwrap();
        p
    }

    const CARD: &str = "Transaction Date,Post Date,Description,Category,Type,Amount,Memo\n\
        07/15/2026,07/16/2026,ACME CLINIC,Health & Wellness,Sale,-42.10,\n";

    #[test]
    fn finds_csvs_and_skips_hidden_and_non_csv() {
        let dir = scratch("find_csvs");
        write(&dir, "activity.csv", CARD);
        write(&dir, "UPPER.CSV", CARD);
        write(&dir, "notes.txt", "hello");
        write(&dir, ".hidden.csv", CARD);
        write(&dir, "sub/nested.csv", CARD);

        let flat = find_csvs(&dir, false).unwrap();
        let names: Vec<String> = flat
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["UPPER.CSV", "activity.csv"]);

        let deep = find_csvs(&dir, true).unwrap();
        assert_eq!(deep.len(), 3, "recursive should pick up sub/nested.csv");
    }

    #[test]
    fn classifies_by_header_not_filename() {
        let dir = scratch("classify");
        let chase = write(&dir, "download (3).csv", CARD);
        let other = write(
            &dir,
            "Chase_statement_totally.csv",
            "Date,Memo,Debit,Credit\n",
        );
        let empty = write(&dir, "empty.csv", "");

        assert_eq!(classify(&chase), Some(Variant::CardModern));
        assert_eq!(classify(&other), None, "name says Chase, header does not");
        assert_eq!(classify(&empty), None);
    }

    #[test]
    fn binary_and_missing_files_are_skipped_not_fatal() {
        let dir = scratch("binary");
        let bin = dir.join("scan.csv");
        std::fs::write(&bin, [0xff_u8, 0xfe, 0x00, 0x01, 0x80]).unwrap();
        assert_eq!(classify(&bin), None);
        assert_eq!(classify(&dir.join("nope.csv")), None);
    }

    #[test]
    fn missing_directory_is_not_found() {
        let err = find_csvs(Path::new("/nonexistent/inbox/xyz"), false).unwrap_err();
        assert_eq!(err.exit_code(), 4);
    }
}
