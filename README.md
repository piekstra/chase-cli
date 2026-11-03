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
| `mcp [--list-tools\|--http --port N\|--new-token]` | read-only tools for Claude ([below](#use-with-claude)) |
| `self-update [--check]` | `self-update/v1` |
| `completions <shell>`, `info` | — |

Global flags: `--json`, `-v/--verbose`, `-q/--quiet`, `--no-color`,
`--config <PATH>`. List commands take `--limit N`, `--since`, `--until` (ISO
`YYYY-MM-DD`); `--limit` returns the *most recent* N. `--strict` turns skipped
rows into a hard failure.

Exit codes: `0` ok · `2` usage · `3` `mcp --http` without a bearer token · `4`
file/directory not found · `5` not a recognized Chase export, or `--strict` with
unparseable rows.

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

## Use with Claude

`chse mcp` serves two read-only tools over MCP, so Claude can answer questions about
the Chase activity you downloaded. There is no hosted service and no connection to
Chase: the tools run on your computer, in your copy of `chse`, and read the CSV files in
one folder.

| Tool | Answers |
| --- | --- |
| `chase_transactions` | What was spent over a period, on what, at which merchant? Rows newest first, with the count and net total of everything matching across pages, narrowed by `since`/`until`, `description`, `category`, `kind` or `file` |
| `chase_exports` | Which accounts and dates do my files cover? Each export's name, type, row count and date span, and how many CSV files were not Chase's |

No tool can move money or change anything: there is no Chase account behind them, only
your files. No tool takes a path. They read the folder you set with `chse config set
inbox <folder>`, read again on each call, so a new download or a new setting applies at
once. `file` picks from the names `chase_exports` gives, and `recursive` reaches that
folder's subfolders.

An answer says what it could not cover. `data_through` is the last date in your files,
and nothing later is known. A row or a whole export that could not be read makes
`complete` false, with a count or the file's name, and never the row's contents. A row
identical to one in another export is counted in `possible_duplicates`, because it may be
one transaction downloaded twice, or two that look alike on different accounts. Rows are
not removed, so `total_amount` counts both; `file` reads one export.

A run of 8 or more digits in a description, memo or file name comes back as its last four
(`****1234`), since a checking export's descriptions can carry an account number. Results
are concise by default (`response_format: "detailed"` adds the post date, memo, Chase's
type, the running balance, the check number and the file), 25 rows a page with a cursor
for more. Descriptions come from Chase and other parties; the server tells Claude to
treat them as data, not instructions.

**Set the folder first.** [Download CSV activity](#getting-the-data-out-of-chase) into a
folder and run `chse config set inbox <folder>`. Until then, the tools say so.

**Claude Code.** Add the `chase-cli` plugin from the
[piekstra plugin marketplace](https://github.com/piekstra/claude-plugins):

```console
$ claude plugin marketplace add piekstra/claude-plugins
$ claude plugin install chase-cli@piekstra-plugins
```

or register the server yourself:

```console
$ claude mcp add chase -- chse mcp
```

**Claude Desktop.** Download `chse-<version>.mcpb` from the
[latest release](https://github.com/piekstra/chase-cli/releases/latest) and open it. It
asks for the folder, and no account or password. The bundled `chse` touches no keychain.

Any other MCP client that launches a local server can run `chse mcp`. `$CHSE_INBOX` wins
over the config for these tools, and is how the extension passes its folder; the `import`
commands do not read it.

**Claude on the web and mobile.** claude.ai reaches tools through a remote connector, and
your files stay on your computer, so `chse` serves the same tools from your computer to a
connector you deploy into your own Cloudflare account:

```console
$ chse mcp --new-token            # once: a bearer token, saved in the keychain and printed for the Worker
$ chse mcp --http --port 8738     # serve on 127.0.0.1 only
```

The connector, [cli-common's `connector/`](https://github.com/piekstra/cli-common/tree/main/connector#tools-from-your-computer-home-modules),
reaches that port through a Cloudflare Tunnel in your account; its README walks through
the tunnel, the VPC service, a launchd agent that keeps the server running, and the
`HOME_CHASE_TOKEN` secret, with the threat model. `--http` binds 127.0.0.1 only, refuses
to start without the token, refuses any request whose `Host` is not loopback or that
carries an `Origin`, and serves the read-only tools only. Your files stay on your
computer, but tool results do not: your transactions pass through Cloudflare and reach
Claude ([`SECURITY.md`](SECURITY.md)). While the computer sleeps, the tools answer that
it is offline.

After `chse self-update`, restart the server: a running one keeps the old code, and its
launchd agent restarts it only when it exits. Its start line names the version.

```console
$ launchctl kickstart -k gui/$(id -u)/com.piekstra.chse-mcp-http
```

`self-update` does not reach a Claude Desktop extension, which runs its own copy. Install
the new release's `.mcpb` too.

## Security

No Chase credential and no connection to Chase, so there is no bank login to leak. The
commands read files you point them at, and the MCP tools read the folder you configured.
Two paths reach the network: `self-update`, to GitHub, and `mcp --http`, which listens on
127.0.0.1 for your own connector. `mcp --new-token` stores one keychain item, that
connector's bearer token. Every fixture in this repo is synthetic and enforced as such by
a test; no real transaction, merchant, balance, or account number has ever been committed.
See [`SECURITY.md`](SECURITY.md).

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
