# AGENTS.md — working on `chase-cli`

Canonical contributor/agent notes for this repo. `CLAUDE.md` points here.

## What this is

A `piekstra-cli/1` family CLI (binary `chse`) that parses Chase's own CSV
activity exports into family transaction DTOs. It is a thin layer over the
shared `pk-cli-*` crates from
[`cli-common`](https://github.com/piekstra/cli-common), pinned to tag
**v0.21.0**. Read that repo's `DESIGN.md`, `conformance.md`, and `PROFILES.md`
before changing the surface — do not reconstruct the spec from memory.

## The two rules that matter most here

**1. There is no portal client, and adding one is a deliberate decision, not a
task.** Chase removed third-party screen scraping in 2022 and gates data access
behind contracted (now paid) aggregators. A speculative HTTP client against
chase.com risks a `403` at best and a **lockout on the household's primary bank
account** at worst. If you are ever asked to add one: read
[`docs/api.md`](docs/api.md) first, prefer a real browser over an HTTP client,
attempt **once**, stop at the first anomaly, and never vary fingerprints to
evade detection. Adding auth is a behavior change that must update
`docs/api.md`, `README.md`, `info`, and this file in the same commit.

**2. Never commit real financial data.** Treat this repo as public regardless
of its visibility. Every fixture is invented, not captured — see
[`tests/fixtures/README.md`](tests/fixtures/README.md), enforced by
`fixture_shapes.rs`. If you need a new format case, write new synthetic rows;
do not paste from a real export, not even "just to check".

## Layout

| Path | Owns |
|---|---|
| `src/main.rs` | clap tree, exit-code mapping, `info` |
| `src/format.rs` | export variants + header-signature detection |
| `src/value.rs` | cell normalization: dates → ISO, amounts → integer cents, description cleanup, kind mapping |
| `src/parser.rs` | CSV → `Transaction`, row-error collection, range filtering |
| `src/scan.rs` | directory walk + cheap one-line classification |
| `src/dto.rs` | output shapes and their table views |
| `src/commands/import.rs` | the `import` command group |
| `src/config.rs` | non-secret settings (`inbox`) |

## Invariants

- **`--json` on every command**, exactly one JSON document per invocation, DTO
  alone on stdout. Diagnostics and warnings go to **stderr**, always.
- **Money never touches a float.** Parse to integer cents, emit a string
  decimal. `Money::from_cents` does the formatting.
- **Spend-positive.** Chase's signs are inverted relative to the family
  contract; `parser.rs` negates. Running balances are not negated. If you touch
  this, `signs_are_normalized_to_spend_positive` is the test that guards it.
- **ISO dates everywhere** on flags and in output.
- **Validate args before touching the filesystem**, so `--help` and bad input
  never read a file.
- **A malformed row is skipped and reported, not fatal** — one bad line should
  not cost the other 399. `--strict` opts into hard failure.
- Field/column order is insertion order and is part of the contract;
  `Transaction`'s first four fields deliberately mirror
  `pk_cli_utility::Transaction`.

## Reuse before you write

Grep `~/Dev/cli-common/crates` before adding a helper. Already provided and
easy to duplicate by accident: `pk_cli_core::dates` (`parse_mm_slash_dd_yyyy`,
`fmt_iso`, `parse_iso`), `output::{emit, table, table_view, kv, json, fail}`,
`Money`, `Paged`, `RangeArgs` (`--limit`/`--since`/`--until`), `CliError` and
its exit codes, `ConfigStore`, `AuthStatus`, `Updater`. A genuinely reusable
mechanism belongs in `cli-common`, not copy-pasted here.

## The MCP server

`src/mcp.rs` serves `chase_transactions` and `chase_exports` over the parser the commands
use (cli-common `DESIGN.md` §1.9). Three rules hold there, and `src/mcp/tests.rs` holds
them to it: no tool takes a path, no result carries the inbox's own path or a run of 8+
digits, and a skipped row is counted and never quoted. New text from a file goes through
`mask_digit_runs`. Tools stay read-only. Adding a tool also changes the tool list in
`packaging/mcpb/manifest.json`, which `tests/packaging.rs` checks, and in
`claude-plugins`.

## Tests

`cargo test --all` — fully offline, no mocking library.

- unit tests inline per module (detection, date/amount edge cases, row mapping)
- `tests/cli_surface.rs` — black-box `assert_cmd`: every subcommand's `--help`
  renders (catches a flag colliding with a global like `-q`), exit codes, the
  `--json` error DTO, `info`, `auth status`, completions
- `tests/fixture_shapes.rs` — contract tests over the synthetic fixtures,
  asserting the fields the table views actually read, plus the fixture-data
  policy
- `src/mcp/tests.rs` — the tools, in process, over those fixtures and scratch folders
- `tests/mcp_stdio.rs`, `tests/mcp_http.rs` — the real binary over each transport
- `tests/packaging.rs` — the `.mcpb` manifest against the binary it packages

Tests redirect `XDG_CONFIG_HOME` and use the OS temp dir, so they never read or
clobber a developer's real `chse` config or inbox.

## Shipping

`make verify` (fmt-check + clippy + test + smoke) is the gate; CI runs the same
on **Linux only** — this CLI holds no provider credential, and only `mcp --http`
opens a keychain, for its bearer token, so a macOS runner would exercise nothing
extra. To release: bump `version` in `Cargo.toml` and
`packaging/mcpb/manifest.json`, add to `CHANGELOG.md`, merge, and push a `v<version>`
tag. The workflow builds `chse-<target>.tar.gz` assets that `self-update` can find and
the `.mcpb`, and publishes the release once all of them are there. A pull request that
touches the dependencies or the packaging runs the same builds as a dry run.

Done means installed and driven, not compiled — `make install` and run the real
binary across the surface, including `--json` and the error paths.

## Not registered in cli-mcp

Intentionally. If that changes, add the `CLI_CONFIG` entry, the `chase_cli`
tool function, and the CLI-name lists in `~/Dev/open-cli-mcp/cli-mcp`.
