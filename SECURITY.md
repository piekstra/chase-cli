# Security

## Threat model, briefly

`chse` is an offline file parser. It holds **no credential**, writes **no
keychain entry**, and makes **no network request** except when you explicitly
run `self-update` (which talks to the GitHub releases API, not to Chase). It
reads only the files and directories you point it at.

That is a deliberate design choice, not an accident of scope — see
[`docs/api.md`](docs/api.md).

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

## If a credential is ever added

Adding a portal session changes this document's premise. It would land with:
secrets only from the OS keychain (`piekstra.chse`), stdin, or a named env var
— never argv, never a file, never a log — and `--verbose` still never printing
cookies or full account numbers.

## Reporting

This is a personal-family tool. Open an issue, or for anything sensitive,
contact the repository owner directly rather than filing publicly.
