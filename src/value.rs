//! Cell-level normalization: Chase's date, amount, and description spellings
//! into the family's ISO / string-decimal / cleaned-text conventions.

use pk_cli_core::dates;

/// Strip a BOM, surrounding quotes, and padding from a raw CSV cell.
fn clean_cell(raw: &str) -> &str {
    raw.trim_start_matches('\u{feff}')
        .trim()
        .trim_matches('"')
        .trim()
}

/// Parse any date spelling seen in a Chase export into ISO `YYYY-MM-DD`.
///
/// Chase itself writes `MM/DD/YYYY`, but exports reach us after a round trip
/// through Excel/Sheets often enough that the other shapes are worth
/// accepting: a spreadsheet will happily rewrite the column as `YYYY-MM-DD`,
/// `M/D/YY`, or a long-form date, and silently dropping those rows would be a
/// quiet data-loss bug in a tax pipeline.
pub fn parse_date(raw: &str) -> Option<String> {
    let s = clean_cell(raw);
    if s.is_empty() {
        return None;
    }

    // Chase's native spelling first, via the shared helper (which range-checks).
    if let Some(c) = dates::parse_mm_slash_dd_yyyy(s) {
        return Some(dates::fmt_iso(c));
    }

    if let Some(c) = numeric_date(s, '/').or_else(|| numeric_date(s, '-')) {
        return Some(dates::fmt_iso(c));
    }
    long_form_date(s).map(dates::fmt_iso)
}

/// `M/D/YY`, `M/D/YYYY`, `YYYY-MM-DD`, `MM-DD-YYYY` — separator supplied.
fn numeric_date(s: &str, sep: char) -> Option<(i64, u32, u32)> {
    let parts: Vec<&str> = s.split(sep).collect();
    if parts.len() != 3 || !parts.iter().all(|p| is_all_digits(p)) {
        return None;
    }
    // A 4-digit leading component means ISO order; otherwise month-first.
    let (y, m, d) = if parts[0].len() == 4 {
        (parts[0].parse().ok()?, parts[1], parts[2])
    } else {
        (expand_year(parts[2])?, parts[0], parts[1])
    };
    let m: u32 = m.parse().ok()?;
    let d: u32 = d.parse().ok()?;
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// `January 15, 2024` / `Jan 15 2024`.
fn long_form_date(s: &str) -> Option<(i64, u32, u32)> {
    let cleaned = s.replace(',', " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let m = month_from_name(parts[0])?;
    let d: u32 = parts[1].parse().ok()?;
    let y: i64 = parts[2].parse().ok()?;
    (parts[2].len() == 4 && (1..=31).contains(&d)).then_some((y, m, d))
}

fn month_from_name(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let lower = name.to_lowercase();
    MONTHS
        .iter()
        .position(|m| *m == lower || (lower.len() >= 3 && m.starts_with(&lower)))
        .map(|i| i as u32 + 1)
}

/// Two-digit years are 21st-century. Chase card exports predating 2000 are not
/// a case we need, and the alternative (a sliding window) is worse: it would
/// reinterpret archived files differently as the clock moves.
fn expand_year(raw: &str) -> Option<i64> {
    match raw.len() {
        2 => raw.parse::<i64>().ok().map(|y| 2000 + y),
        4 => raw.parse().ok(),
        _ => None,
    }
}

fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Parse a money cell to an exact integer number of cents.
///
/// Integer arithmetic end to end — no float ever touches the value, because a
/// binary float cannot represent most cent amounts exactly and the family
/// output contract is a string decimal. Handles the accretions a money column
/// picks up in the wild: `$`, thousands separators, non-breaking spaces,
/// `(123.45)` accounting negatives, and a trailing minus sign.
pub fn parse_amount_cents(raw: &str) -> Option<i64> {
    let s = clean_cell(raw);
    if s.is_empty() {
        return None;
    }

    let mut negative = false;
    let mut body = s.to_string();

    if body.starts_with('(') && body.ends_with(')') {
        negative = true;
        body = body[1..body.len() - 1].to_string();
    }

    body.retain(|c| !matches!(c, '$' | '€' | '£' | ',' | '\u{a0}' | ' ' | '\t' | '"'));

    if let Some(rest) = body.strip_suffix('-') {
        negative = !negative;
        body = rest.to_string();
    }
    if let Some(rest) = body.strip_prefix('-') {
        negative = !negative;
        body = rest.to_string();
    } else if let Some(rest) = body.strip_prefix('+') {
        body = rest.to_string();
    }

    let (whole, frac) = match body.split_once('.') {
        Some((w, f)) => (w, f),
        None => (body.as_str(), ""),
    };
    // An empty whole part is legal (`.50`); an empty string overall is not.
    if (whole.is_empty() && frac.is_empty())
        || !(whole.is_empty() || is_all_digits(whole))
        || !(frac.is_empty() || is_all_digits(frac))
        || frac.len() > 2
    {
        return None;
    }

    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    // `12.5` means 50 cents, not 5.
    let cents: i64 = match frac.len() {
        0 => 0,
        1 => frac.parse::<i64>().ok()? * 10,
        _ => frac.parse().ok()?,
    };
    let total = whole.checked_mul(100)?.checked_add(cents)?;
    Some(if negative { -total } else { total })
}

/// Tidy a description without rewriting it.
///
/// Deliberately conservative: collapse whitespace and decode the HTML entities
/// that show up when a description round-trips through a web view. We do *not*
/// do merchant-name normalization (stripping `SQ *`, store numbers, city/state
/// tails) — that is lossy, and a downstream tax scanner is better served by
/// the provider's own text plus its `Category`.
pub fn clean_description(raw: &str) -> String {
    let decoded = decode_entities(clean_cell(raw));
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    const ENTITIES: [(&str, &str); 9] = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&#x2F;", "/"),
        ("&#47;", "/"),
    ];
    let mut out = s.to_string();
    // `&amp;` last would double-decode `&amp;lt;`; decoding it first is also
    // wrong for that input, so decode the non-ampersand entities first and
    // `&amp;` at the end.
    for (from, to) in ENTITIES.iter().skip(1) {
        out = out.replace(from, to);
    }
    out.replace("&amp;", "&")
}

/// Normalized transaction kind for a **card** row, from Chase's `Type` column.
///
/// `raw_cents` is Chase's own sign (purchases negative) and is only a fallback
/// for exports whose `Type` is blank.
pub fn card_kind(type_col: Option<&str>, raw_cents: i64) -> String {
    match type_col.map(|t| t.trim().to_lowercase()).as_deref() {
        Some("sale") => "charge",
        Some("payment") => "payment",
        Some("return") | Some("refund") => "refund",
        Some("fee") => "fee",
        Some("adjustment") => "adjustment",
        _ if raw_cents <= 0 => "charge",
        _ => "payment",
    }
    .to_string()
}

/// Normalized transaction kind for a **checking/savings** row, from the
/// `Details` column (`DEBIT` / `CREDIT` / `CHECK` / `DSLIP`).
pub fn checking_kind(details: Option<&str>, raw_cents: i64) -> String {
    match details.map(|d| d.trim().to_uppercase()).as_deref() {
        Some("DEBIT") => "debit",
        Some("CREDIT") => "credit",
        Some("CHECK") => "check",
        Some("DSLIP") => "deposit",
        _ if raw_cents < 0 => "debit",
        _ => "credit",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chase_native_dates() {
        assert_eq!(parse_date("07/15/2026").as_deref(), Some("2026-07-15"));
        assert_eq!(parse_date("12/01/2025").as_deref(), Some("2025-12-01"));
    }

    #[test]
    fn parses_spreadsheet_mangled_dates() {
        assert_eq!(parse_date("7/5/26").as_deref(), Some("2026-07-05"));
        assert_eq!(parse_date("2026-07-15").as_deref(), Some("2026-07-15"));
        assert_eq!(parse_date("07-15-2026").as_deref(), Some("2026-07-15"));
        assert_eq!(
            parse_date("January 15, 2026").as_deref(),
            Some("2026-01-15")
        );
        assert_eq!(parse_date("Jan 5 2026").as_deref(), Some("2026-01-05"));
    }

    #[test]
    fn tolerates_bom_quotes_and_padding_on_dates() {
        assert_eq!(
            parse_date("\u{feff}\"07/15/2026\" ").as_deref(),
            Some("2026-07-15")
        );
    }

    #[test]
    fn rejects_nonsense_dates() {
        assert_eq!(parse_date(""), None);
        assert_eq!(parse_date("   "), None);
        assert_eq!(parse_date("not a date"), None);
        assert_eq!(parse_date("13/45/2026"), None);
        assert_eq!(parse_date("07/15"), None);
    }

    #[test]
    fn parses_amounts_as_exact_cents() {
        assert_eq!(parse_amount_cents("-12.34"), Some(-1234));
        assert_eq!(parse_amount_cents("1234.56"), Some(123456));
        assert_eq!(parse_amount_cents("$1,234.56"), Some(123456));
        assert_eq!(parse_amount_cents("0.00"), Some(0));
        assert_eq!(parse_amount_cents("100"), Some(10000));
        assert_eq!(parse_amount_cents("12.5"), Some(1250));
        assert_eq!(parse_amount_cents(".50"), Some(50));
    }

    #[test]
    fn parses_accounting_negatives() {
        assert_eq!(parse_amount_cents("(12.34)"), Some(-1234));
        assert_eq!(parse_amount_cents("($1,200.00)"), Some(-120000));
        assert_eq!(parse_amount_cents("12.34-"), Some(-1234));
    }

    /// The classic float bug: 0.1 + 0.2 style representation error. Cents are
    /// integers here, so amounts that are unrepresentable in binary floating
    /// point survive a round trip exactly.
    #[test]
    fn amounts_are_exact_not_floating_point() {
        assert_eq!(parse_amount_cents("0.10"), Some(10));
        assert_eq!(parse_amount_cents("0.20"), Some(20));
        assert_eq!(parse_amount_cents("8398.51"), Some(839851));
        assert_eq!(parse_amount_cents("1.005"), None); // 3 decimals is not money
    }

    #[test]
    fn rejects_non_amounts() {
        assert_eq!(parse_amount_cents(""), None);
        assert_eq!(parse_amount_cents("abc"), None);
        assert_eq!(parse_amount_cents("1.2.3"), None);
        assert_eq!(parse_amount_cents("--5"), None);
    }

    #[test]
    fn cleans_descriptions_conservatively() {
        assert_eq!(clean_description("  ACME   CLINIC  "), "ACME CLINIC");
        assert_eq!(
            clean_description("BOB &amp; SONS PLUMBING"),
            "BOB & SONS PLUMBING"
        );
        assert_eq!(clean_description("TOM&#39;S HARDWARE"), "TOM'S HARDWARE");
        // Merchant prefixes/suffixes are preserved, not stripped.
        assert_eq!(
            clean_description("SQ *ACME CLINIC 555-1234"),
            "SQ *ACME CLINIC 555-1234"
        );
    }

    #[test]
    fn maps_card_kinds() {
        assert_eq!(card_kind(Some("Sale"), -1234), "charge");
        assert_eq!(card_kind(Some("Payment"), 50000), "payment");
        assert_eq!(card_kind(Some("Return"), 1234), "refund");
        assert_eq!(card_kind(Some("Adjustment"), 100), "adjustment");
        // Blank Type falls back to Chase's own sign.
        assert_eq!(card_kind(None, -1234), "charge");
        assert_eq!(card_kind(Some(""), 1234), "payment");
    }

    #[test]
    fn maps_checking_kinds() {
        assert_eq!(checking_kind(Some("DEBIT"), -1234), "debit");
        assert_eq!(checking_kind(Some("CREDIT"), 1234), "credit");
        assert_eq!(checking_kind(Some("CHECK"), -5000), "check");
        assert_eq!(checking_kind(Some("DSLIP"), 10000), "deposit");
        assert_eq!(checking_kind(None, -1), "debit");
    }
}
