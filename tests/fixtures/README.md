# Test fixtures — synthetic Chase CSV exports

**Nothing here was captured from a real account.** Every row was written by
hand to reproduce a documented quirk of Chase's export formats. There is no
scrubbed-capture path in this repo to get wrong: no real transaction, merchant,
balance, account number, or card number has ever been committed, and none
should be.

Written 2026-08-12 against the format notes in [`docs/api.md`](../../docs/api.md).

## Policy, enforced by `fixture_shapes.rs`

Stated positively, so this file never has to name the things it exists to keep
out:

- Every `Description` value must either contain a synthetic marker — `ACME` or
  `EXAMPLE` — or be one of the bank-generated literals a real export contains
  regardless of who owns the account (`PAYMENT THANK YOU`,
  `ANNUAL MEMBERSHIP FEE`, `INTEREST CHARGE`, `CHECK #`, `PAYROLL DIRECT DEP`).
- No run of 8 or more consecutive digits anywhere in any fixture — that is the
  shape of an account, card, or routing number.
- No `@`, so no address can hide in a memo field.

Amounts and balances are invented round-ish numbers. Dates are in the future
(2026) or long past (2019, for the legacy format) precisely so they cannot be
mistaken for a real statement period.

## What each fixture covers

| File | Variant | Exercises |
|---|---|---|
| `card-modern.csv` | `card-modern` | The current 7-column export. Quoted description containing a comma, `"-1,250.00"` thousands separator, `&amp;` entity, populated and blank `Memo`, blank `Category` on a payment, and all four `Type` values (`Sale`, `Return`, `Payment`, `Fee`). Categories are the tax-relevant ones — `Health & Wellness`, `Home`. |
| `card-modern-no-memo.csv` | `card-modern` | The same export without the trailing `Memo` column, which some accounts and date ranges omit. |
| `card-legacy.csv` | `card-legacy` | The older 5-column export: `Type` first, `Trans Date` rather than `Transaction Date`, and no `Category` at all. |
| `checking.csv` | `checking` | Deposit-account export: `Details` (`DEBIT`/`CREDIT`/`CHECK`), running `Balance`, `Check or Slip #`, and the trailing comma that gives every row a ragged final column. |
| `card-spreadsheet-roundtrip.csv` | `card-modern` | A card export after a trip through a spreadsheet: UTF-8 BOM, ISO dates, `M/D/YY` dates, a quoted `"May 15, 2026"` long-form date, `($142.10)` accounting negative, `"-$1,385.00"` with symbol and separator, and a `24.99-` trailing minus. |
| `malformed-rows.csv` | `card-modern` | One good row and three that must be skipped with a row-numbered message: an unparseable date, a missing description, and a non-numeric amount. |
| `not-chase.csv` | — | A foreign bank's CSV. `import scan` must not claim it; `import csv` must exit 5. |
| `nested/card-nested.csv` | `card-modern` | Lets `import scan --recursive` prove it descends, and that the default does not. |
