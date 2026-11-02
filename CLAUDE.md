# CLAUDE.md

See [`AGENTS.md`](AGENTS.md) — it is the canonical guide for this repo.

Two things to know before you touch anything:

1. **There is no Chase portal client, deliberately.** Do not add one casually:
   Chase removed third-party screen scraping in 2022, and a speculative login
   risks locking the household's primary bank account. Read
   [`docs/api.md`](docs/api.md) first.
2. **Never commit real financial data.** Fixtures are synthetic and a test
   enforces it. Treat this repo as public.
