//! Chase CSV export variants, and detection by header signature.
//!
//! Chase hands out several differently-shaped CSVs depending on the account
//! and on when the export was taken. We identify a file by its header row
//! alone, so `import scan` can classify a directory cheaply (read one line per
//! file) and so an unrecognized file fails loudly instead of being coerced
//! into the wrong column mapping.
//!
//! Detection matches **whole normalized column names**, never substrings of a
//! joined header blob. The substring approach is the tempting one and it is a
//! trap: `"transaction date"` and `"trans date"` are different columns that
//! distinguish the modern card export from the legacy one, and a careless
//! `contains` chain quietly folds one into the other.

/// A recognized Chase CSV export shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Current credit-card activity export.
    /// `Transaction Date,Post Date,Description,Category,Type,Amount,Memo`
    /// (the trailing `Memo` column is absent on some accounts/date ranges).
    CardModern,
    /// Legacy credit-card export, `Type` first and no `Category`:
    /// `Type,Trans Date,Post Date,Description,Amount`
    CardLegacy,
    /// Checking / savings activity export:
    /// `Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #`
    Checking,
}

impl Variant {
    /// Stable slug used in DTOs (`variant` field) — part of the output contract.
    pub fn as_str(self) -> &'static str {
        match self {
            Variant::CardModern => "card-modern",
            Variant::CardLegacy => "card-legacy",
            Variant::Checking => "checking",
        }
    }

    /// Whether this variant describes a credit-card account (vs. deposit).
    pub fn is_card(self) -> bool {
        matches!(self, Variant::CardModern | Variant::CardLegacy)
    }
}

/// Normalize one header cell for comparison: strip a UTF-8 BOM, trim, drop
/// surrounding quotes, lowercase, and collapse internal whitespace runs.
///
/// The BOM matters: Chase exports are frequently UTF-8-with-BOM, and an
/// unstripped BOM makes the very first column name `"\u{feff}transaction
/// date"`, which silently defeats detection on the *first* column only — the
/// kind of bug that looks like "Chase changed their format".
pub fn normalize(header: &str) -> String {
    let trimmed = header.trim_start_matches('\u{feff}').trim();
    let unquoted = trimmed.trim_matches('"').trim();
    unquoted
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Identify a Chase export from its header row, or `None` if it isn't one.
pub fn detect(headers: &[String]) -> Option<Variant> {
    let cols: Vec<String> = headers.iter().map(|h| normalize(h)).collect();
    let has = |name: &str| cols.iter().any(|c| c == name);

    // Checking first: `details` + `posting date` are unique to it, and it also
    // carries `description`/`amount`/`type`, which would otherwise let a
    // looser card rule claim it.
    if has("details") && has("posting date") && has("description") && has("amount") {
        return Some(Variant::Checking);
    }
    if has("transaction date") && has("post date") && has("description") && has("amount") {
        return Some(Variant::CardModern);
    }
    if has("trans date") && has("post date") && has("description") && has("amount") {
        return Some(Variant::CardLegacy);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(line: &str) -> Vec<String> {
        line.split(',').map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_modern_card_export() {
        let h = headers("Transaction Date,Post Date,Description,Category,Type,Amount,Memo");
        assert_eq!(detect(&h), Some(Variant::CardModern));
    }

    #[test]
    fn detects_modern_card_export_without_memo() {
        let h = headers("Transaction Date,Post Date,Description,Category,Type,Amount");
        assert_eq!(detect(&h), Some(Variant::CardModern));
    }

    #[test]
    fn detects_legacy_card_export() {
        let h = headers("Type,Trans Date,Post Date,Description,Amount");
        assert_eq!(detect(&h), Some(Variant::CardLegacy));
    }

    #[test]
    fn detects_checking_export() {
        let h = headers("Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #");
        assert_eq!(detect(&h), Some(Variant::Checking));
    }

    /// The modern and legacy card exports differ only in `Transaction Date` vs
    /// `Trans Date`; whole-name matching must keep them apart.
    #[test]
    fn modern_and_legacy_are_not_confused() {
        let modern = headers("Transaction Date,Post Date,Description,Category,Type,Amount,Memo");
        let legacy = headers("Type,Trans Date,Post Date,Description,Amount");
        assert_eq!(detect(&modern), Some(Variant::CardModern));
        assert_eq!(detect(&legacy), Some(Variant::CardLegacy));
    }

    #[test]
    fn tolerates_bom_quotes_case_and_padding() {
        let h =
            headers("\u{feff}\"TRANSACTION DATE\", Post  Date ,Description,Category,Type,Amount");
        assert_eq!(detect(&h), Some(Variant::CardModern));
    }

    /// Chase's checking export ends with a trailing comma, so the parsed
    /// header row carries an empty final column. That must not break detection.
    #[test]
    fn tolerates_trailing_empty_column() {
        let h = headers("Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #,");
        assert_eq!(detect(&h), Some(Variant::Checking));
    }

    #[test]
    fn rejects_foreign_exports() {
        // A different bank's CSV, and a spreadsheet that merely has dates.
        assert_eq!(
            detect(&headers("Date,Description,Amount,Running Bal.")),
            None
        );
        assert_eq!(detect(&headers("Order ID,Order Date,Product Name")), None);
        assert_eq!(detect(&headers("date,amount")), None);
        assert_eq!(detect(&[]), None);
    }
}
