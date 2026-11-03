# Contributing

## Setup

```sh
git clone https://github.com/piekstra/chase-cli
cd chase-cli
make deps
make verify
```

Rust stable (edition 2021, MSRV 1.88). Shared behavior comes from the
`pk-cli-*` crates in [cli-common](https://github.com/piekstra/cli-common),
pinned to a **tag** — never a path or branch, and every `pk-cli-*` on the same
tag.

## The loop

```sh
make dev          # debug build, re-signed on macOS so keychain grants survive
make test         # offline; no network, no keychain, no real account
make verify       # fmt-check + clippy + test + smoke — the CI gate
make install      # then actually drive the installed binary
```

`make verify` passing is necessary, not sufficient. Install and run the real
binary across the surface — every command, `--json`, and the error paths.
"Done" means installed, not compiled: a stale `~/.cargo/bin/chse` will happily
serve old behavior.

## House rules

Read [`AGENTS.md`](AGENTS.md) for the full set. The short version:

- **Never commit real financial data.** Fixtures are synthetic and a test
  enforces it. Adding a format case means writing new synthetic rows.
- **Don't add a Chase portal client casually.** See
  [`docs/api.md`](docs/api.md) — the risk is an account lockout, not a failed
  request.
- `--json` on every command; one JSON document per invocation; data on stdout,
  diagnostics on stderr.
- Money as integer cents internally, string decimal on the wire. No floats.
- ISO `YYYY-MM-DD` on every date flag and every emitted date.
- Reuse `cli-common` before writing a helper; grep `~/Dev/cli-common/crates`.

## Adding support for a format quirk

1. Add a **synthetic** fixture row (or a new fixture) reproducing it.
2. Add a unit test in the module that owns the behavior — `format.rs` for
   header detection, `value.rs` for a cell spelling, `parser.rs` for row
   mapping.
3. Add a contract assertion in `tests/fixture_shapes.rs` if it changes what a
   table view reads.
4. Document it in [`docs/api.md`](docs/api.md) §3 "The traps" so nobody
   rediscovers it.

## Releases

Bump `version` in `Cargo.toml` and merge to `main`. The release workflow tags
`v<version>` and publishes `chse-<target>.tar.gz` + `.sha256` assets — the
names `chse self-update` expects. No manual tagging.
