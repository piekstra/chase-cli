# chase-cli (`chse`)

Turn Chase's own CSV activity exports into clean, schema-tagged transaction
data — for feeding a tax-deduction scan (medical, rental-property services)
without hand-editing spreadsheets.

Unofficial; not affiliated with or endorsed by JPMorgan Chase & Co.

Conforms to the `piekstra-cli/1` spec ([cli-common](https://github.com/piekstra/cli-common)).

> **Offline-first, and there is no portal login.** JPMorgan Chase publishes no
> personal-banking API a customer can self-serve against, and finished
> eliminating third-party screen scraping in 2022. So this CLI does the thing
> that actually works, completely: it parses the export Chase hands you.
> The full reasoning, research, and the realistic future paths are in
> [`docs/api.md`](docs/api.md).

## Install

```sh
make install     # cargo install --path . --force, then re-sign on macOS
```

## Getting the data out of Chase

Sign in → pick the account → **Activity** → the download icon → **CSV**. Card
accounts also offer statement PDFs and `.QFX`/`.OFX`/`.QBO`; this tool reads
the CSV.

## Quickstart

```sh
# Parse one export.
chse import csv ~/Downloads/Chase_Activity.csv

# Just the 2025 tax year, as JSON.
chse import csv activity.csv --since 2025-01-01 --until 2025-12-31 --json

# What Chase exports are sitting in the Drive inbox?
chse config set inbox ~/Drive/inbox
chse import scan
chse import scan --recursive --transactions --json   # merged, newest first

# Why didn't that file parse?
chse import inspect weird.csv
```

## Commands

| Command | Emits |
|---|---|
| `import csv <FILE>` | `transaction-list/v1` |
| `import scan [DIR]` | `chase-csv-file-list/v1` — Chase-shaped CSVs found by header signature |
| `import scan --transactions` | `transaction-list/v1`, merged across every file found |
| `import inspect <FILE>` | `chase-csv-file/v1` — variant, counts, date span, per-row errors |
| `auth status` | `auth-status/v1` — always `required: false` |
| `config path\|show\|set\|unset` | non-secret settings (`inbox`) |
| `self-update [--check]` | `self-update/v1` |
| `completions <shell>`, `info` | — |

Global flags: `--json`, `-v/--verbose`, `-q/--quiet`, `--no-color`,
`--config <PATH>`. List commands take `--limit N`, `--since`, `--until` (ISO
`YYYY-MM-DD`); `--limit` returns the *most recent* N. `--strict` turns skipped
rows into a hard failure.

Exit codes: `0` ok · `2` usage · `4` file/directory not found · `5` not a
recognized Chase export, or `--strict` with unparseable rows.

## Formats handled

Four export variants, detected by header signature — current card exports (with
and without `Memo`), the legacy `Type,Trans Date,…` card export, and
checking/savings. Plus the quirks: UTF-8 BOM, the checking export's trailing
comma, quoted commas, thousands separators, and spreadsheet round-trips (ISO /
`M/D/YY` / long-form dates, `($142.10)` accounting negatives, trailing minus).
See [`docs/api.md` §3](docs/api.md) for the table and the traps.

**The one thing to know:** Chase writes purchases **negative**. This tool emits
the family convention — **charges positive, payments and refunds negative** —
so a `-142.10` clinic charge comes out as `142.10` / `kind: "charge"`. Money is
always an exact string decimal, never a float. Running balances are not flipped.

## Security

No credential, no keychain entry, no network calls — there is nothing to leak.
The tool only ever reads files you point it at. Every fixture in this repo is
synthetic and enforced as such by a test; no real transaction, merchant,
balance, or account number has ever been committed. See
[`SECURITY.md`](SECURITY.md).

## Development

```sh
make verify    # fmt-check + clippy + test + smoke — the CI gate
make test
```

The whole suite is offline: no network, no keychain, no real account.
Contributor notes are in [`AGENTS.md`](AGENTS.md).

## Conformance note

`auth status` is the only `auth` subcommand. Per SPEC §1.2, a credential-free
CLI reports `required: false` / `method: "none"` so drivers need no special
case; `login`/`logout`/`set-credential` would be surface with nothing behind
it. They arrive with the first real session — see
[`docs/api.md` §2](docs/api.md).
