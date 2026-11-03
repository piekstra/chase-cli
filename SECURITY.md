# Security

## Threat model, briefly

`chse` is a file parser with no connection to Chase. It holds **no Chase credential**
and makes **no request to Chase**. It reads only the files and directories you point it
at. Three things reach further, each only when you run it:

- `self-update` talks to the GitHub releases API.
- `mcp --new-token` writes one keychain item (`piekstra.chse`, `mcp-http-token`), a bearer
  token that signs in to nothing at Chase. `mcp --http` reads it.
- `mcp --http` listens on 127.0.0.1 for your own connector; see below.

Not connecting to Chase is a deliberate design choice, not an accident of scope — see
[`docs/api.md`](docs/api.md).

## The MCP server

`chse mcp` serves two read-only tools to an MCP client (Claude Code, Claude Desktop) over
stdio; the server opens no port unless run with `--http` (below). **What the tools return
goes to Claude on every path**, as part of your conversation: transactions with their
dates, amounts, descriptions and categories, and your exports' file names and date spans.
Exports hold the real activity of your accounts, so choose the folder with that in mind.

No tool takes a path. The tools read the `inbox` in your config, or `$CHSE_INBOX`, which
the Claude Desktop extension sets from the folder picked in its settings; the config comes
from `--config` or `~/.config/chse`, and nothing in a working directory is read. A `file`
argument is compared with the names found in that folder and never opened, and an argument
the schema does not name is refused, so what a model passes cannot read another file. A
symbolic link in that folder is followed, as the `import` commands follow it; what you link
there is yours to choose.

No result or error carries the folder's own path. A run of 8 or more digits in a
description, memo, category, type, check number or file name is masked to its last four,
before any filter sees it, so a filter cannot test digits a result would not show. A row
that cannot be parsed is counted and its contents are not repeated. Shorter identifiers
and any other text in your files do reach Claude, and a description can name a person or a
place. No tool can move money or change anything: nothing here talks to Chase. The server
reads the config and the files on each call and holds nothing between calls. A Claude
Desktop extension asks only for the folder and touches no keychain.

`chse mcp --http --port <PORT>` serves the same tools over HTTP for the user's own
Cloudflare Worker, which reaches it through the user's own Cloudflare Tunnel (cli-common's
`connector/`). It binds 127.0.0.1 only, with no flag for another address, and refuses to
start without a bearer token of 32 or more characters, from `CHSE_MCP_TOKEN` or the
keychain item `mcp-http-token` (`piekstra.chse`), which `chse mcp --new-token` makes from
256 random bits and replaces only with `--overwrite`. Every request must carry the token
(compared in constant time) and a `Host` of `127.0.0.1` or `localhost`, and must not carry
an `Origin`; there are no CORS headers. A copy of that token lives in your Cloudflare
Worker as its `HOME_CHASE_TOKEN` secret. Your files stay on your computer: the Worker sends
tool calls and receives results.

**Tool results also cross Cloudflare on this path**, through your tunnel and your Worker,
on their way to Claude. Who holds what, and what a leaked token can do, is in the
connector's
[threat model](https://github.com/piekstra/cli-common/blob/main/connector/README.md#threat-model).

`chse` has no `auth logout`, since it has no sign-in. To revoke the connector's access, run
`chse mcp --new-token --overwrite` and restart the server (`launchctl kickstart -k gui/$(id
-u)/com.piekstra.chse-mcp-http`); the old token works until the restart. To stop serving,
run `launchctl bootout gui/$(id -u)/com.piekstra.chse-mcp-http` and stop the tunnel. To
remove the token on macOS, run `security delete-generic-password -s piekstra.chse -a
mcp-http-token`.

## Handling your own data

- The CSV exports you feed this tool contain real financial data. Keep them out
  of this repository; `.gitignore` blocks stray `*.csv`/`*.pdf`/`*.qfx` at the
  repo root as a backstop, but the real rule is: don't put them here.
- Output goes to stdout. If you redirect it to a file, that file has the same
  sensitivity as the export it came from.
- `--verbose` prints per-row parse errors, and those messages quote the
  offending cell. That is runtime output from your own file; it is never
  written to disk by this tool, but don't paste it into a public issue.

## Repository data policy

This repo is treated as public regardless of its current visibility.

- Every fixture under `tests/fixtures/` is **invented**, never captured. No
  real transaction, merchant, balance, account number, or card number has been
  committed at any point in its history.
- The policy is stated positively in
  [`tests/fixtures/README.md`](tests/fixtures/README.md) and enforced by tests
  in `tests/fixture_shapes.rs`: descriptions must carry a synthetic marker, no
  run of 8+ digits may appear (the shape of an account or card number), and no
  `@` may appear.
- A denylist of real names is deliberately *not* used — it would publish
  exactly what it exists to keep out.

## If a Chase credential is ever added

Adding a portal session changes this document's premise. It would land with:
secrets only from the OS keychain (`piekstra.chse`), stdin, or a named env var
— never argv, never a file, never a log — and `--verbose` still never printing
cookies or full account numbers. The tools above would then read Chase, and this
document's account of what they can reach would change with them.

## Reporting

This is a personal-family tool. Open an issue, or for anything sensitive,
contact the repository owner directly rather than filing publicly.
