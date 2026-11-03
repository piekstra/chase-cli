# Chase: the provider reality, and the CSV formats

The other CLIs in this family document reverse-engineered portal endpoints
here. This one documents **why there are none**, so nobody re-derives the
answer, plus the export formats that stand in for them.

Researched 2026-08-12. Re-check before investing in a portal client.

---

## 1. There is no portal client, on purpose

Three independent facts, any one of which would be enough:

**JPMorgan Chase finished eliminating screen scraping in 2022.** After a
two-year migration it moved every third-party integration onto its own secure
API, reached through contracted aggregators. Credential-based scraping of the
retail site is not a supported path that happens to be rude — it is the
specific thing they spent years removing.
([American Banker, 2022-10-06](https://www.americanbanker.com/news/jpmorgan-chase-says-it-has-fully-eliminated-screen-scraping))

**There is no public personal-banking API a customer can self-serve against.**
JPMorgan publishes developer APIs for commercial, treasury, and payments
clients — contracted institutional products, not something an individual
enables for their own card. The consumer-facing surface is the **Chase Data
Sharing** dashboard, which lets you *grant and revoke* an aggregator's access;
it does not issue you a token for your own tooling.

**Access is gated on being an approved data recipient, and now a paying one.**
Mid-2025 JPMorgan told aggregators it would begin charging for customer-data
access, with pricing steepest for payments-related data; reporting put Plaid's
exposure near $300M/yr, and a JPMorgan–Plaid commercial deal followed. Chase
also reported ~1.89B data requests in a single month with only ~13%
customer-initiated, which is the motivation for the meter.
([Forbes, 2025-07-21](https://www.forbes.com/sites/jeffkauflin/2025/07/21/why-jpmorgan-is-hitting-fintechs-with-stunning-new-fees-for-data-access/) ·
[CNBC, 2025-07-28](https://www.cnbc.com/2025/07/28/jpmorgan-fintech-middlemen-plaid-data-requests-taxing-systems.html) ·
[Bloomberg Law](https://news.bloomberglaw.com/banking-law/jpmorgan-plaid-data-fee-deal-shifts-open-banking-battlefield))

A private CLI is not an approved data recipient and cannot become one.

### The anti-automation posture

Consistent with the above, and the reason a "just try it" HTTP client is worse
than nothing:

- Large banks front their login with commercial bot management (WAF + device
  and TLS fingerprinting + behavioral signals). A scripted client that presents
  a plausible user agent still fails on the signals it cannot see, and the
  failure mode is a `403` — or worse, a silent `200` serving an interstitial.
- **Prior family experience:** a portal client that 403s is worse than no
  client. It looks like a feature, reports failure ambiguously, and invites
  retry loops against an account that is one of the highest-value in the
  household.
- The realistic downside is not a failed request. It is **TOS violation,
  step-up device challenges, fraud flags, and account lockout** on the primary
  banking relationship.

> Uncertainty, stated honestly: the *specific* vendor in Chase's bot stack
> (Akamai vs. other) is not something the public sources settle, and this
> document does not claim it. The conclusion does not depend on which vendor it
> is — it depends on the migration being complete and access being contractual.

### Regulatory context, as of August 2026

Worth knowing because it is the thing that could change the answer, and worth
not over-reading because it currently does not.

- The CFPB finalized the §1033 Personal Financial Data Rights rule in **October
  2024**, which pointed toward customer-directed API access.
- It never took effect as written. In litigation (Bank Policy Institute et al.,
  E.D. Ky.) the CFPB told the court it now views the rule as unlawful; the case
  was stayed, the rule enjoined, and the **April 1, 2026** first compliance date
  passed without becoming an enforcement trigger.
- The Bureau is rewriting it: a revised NPRM went to OIRA on **2026-08-06**, and
  the rewrite explicitly contemplates letting providers **charge** for data
  access.

Net: open banking in the US is, today, a commercial arrangement between
institutions. Nothing in the current posture creates a self-serve path for an
individual's own tooling. Re-read this section if the rewrite finalizes.

### Decision record: the live attempt that was authorized and not made

Live authenticated dogfooding was explicitly authorized for this build, under
strict conditions: credentials only from the family secrets tooling, a single
login attempt, and an immediate stop on any anti-bot or device-challenge
signal.

**No login was attempted.** The gating condition failed first — the credential
tooling was not available in the build environment, and the rule for that case
is to stop rather than improvise a credential path. That is also the right
answer on the merits: §1 says the endpoint a login would probe was removed in
2022, so the most likely outcomes were a block or a device challenge on the
household's primary bank account, in exchange for no durable capability.

If a future attempt happens, the constraints stand: **one** attempt, a real
browser rather than an HTTP client, stop on the first anomaly, and never vary
fingerprints to evade detection — that is out of scope and risks the lockout
this section exists to avoid.

---

## 2. Realistic future paths

| Path | What it needs | Verdict |
|---|---|---|
| **Browser-session capture** (the `xfin` pattern: user logs in by hand, the CLI replays the session) | A capture step, cookie refresh-on-use, and treating "HTML where JSON was expected" and "landed on a login URL" as auth failures | **The only plausible one.** Still fragile: sessions are short, step-up challenges recur, and it remains automated access to a portal that removed third-party automation. Would need an explicit decision to accept lockout risk. |
| **Aggregator** (Plaid / MX / Finicity / Akoya) | Being an approved — and now paying — data recipient, with a commercial agreement | **Closed.** Not available to a household CLI at any effort level. |
| **Manual export** (what this CLI does) | Nothing. Chase already offers CSV activity downloads and PDF statements | **Shipped.** Zero lockout risk, no credential to leak, fully testable offline. |

The honest summary: v0's value is that the boring path is *completely* solved —
every quirk in §3 handled, every row typed, every date ISO, every amount an
exact string decimal — rather than a login that half-works.

### If a portal client is ever added

It would be a behavior change to a CLI currently documented as read-only and
holding no Chase credential, so it lands with: `auth login` / `logout` / `set-credential`,
`AuthMethod::BrowserSession`, `info.auth.required = true`, a macOS CI job for
the keychain backend, and this section rewritten in the **same commit**.

---

## 3. The CSV export formats

This is the real interface. Chase's export UI is Account → Activity →
download; card accounts also offer statement PDFs, and Chase additionally
offers `.QFX`/`.OFX`/`.QBO` for Quicken/QuickBooks (not parsed here — CSV
carries everything we need and is far easier to verify).

### Recognized variants

Detected by **header signature**, matching whole normalized column names.

| Variant | Header |
|---|---|
| `card-modern` | `Transaction Date,Post Date,Description,Category,Type,Amount,Memo` |
| `card-modern` (no memo) | `Transaction Date,Post Date,Description,Category,Type,Amount` |
| `card-legacy` | `Type,Trans Date,Post Date,Description,Amount` |
| `checking` | `Details,Posting Date,Description,Amount,Type,Balance,Check or Slip #` |

Column meanings:

- **`Type`** (card): `Sale` · `Payment` · `Return` · `Fee` · `Adjustment`.
- **`Details`** (checking): `DEBIT` · `CREDIT` · `CHECK` · `DSLIP`.
- **`Type`** (checking): the rail — `ACH_DEBIT`, `ACH_CREDIT`, `DEBIT_CARD`,
  `CHECK_PAID`, `QUICKPAY_DEBIT`, … Kept verbatim as `chase_type`.
- **`Category`** (modern card only): Chase's own spend bucket
  (`Health & Wellness`, `Home`, `Groceries`, …). Blank on payments. This is the
  single most useful column for deduction scanning, so it is never rewritten.
- **`Memo`**: usually blank.

### The traps

Each of these is covered by a fixture and a test; none should be rediscovered.

1. **The sign convention is inverted from the family contract.** Chase writes
   purchases **negative** and payments/credits **positive**. The family
   `Transaction` DTO is the opposite — charges positive, credits negative. The
   parser negates every amount, so `-142.10` in the file becomes `142.10` with
   `kind: "charge"`, and a `2500.00` payment becomes `-2500.00`. Getting this
   backwards inverts every deduction total downstream, silently. The running
   `Balance` on checking exports is an account state, not a spend, and is
   **not** negated.
2. **`Transaction Date` vs `Trans Date` is the only thing separating the modern
   and legacy card formats.** Substring matching on a joined header string
   quietly folds one into the other; match whole column names.
3. **UTF-8 BOM.** Exports frequently carry one. Unstripped, it corrupts the
   *first* column name only — which reads as "Chase changed their format".
4. **The checking export has a trailing comma**, giving every row a ragged
   extra empty column. The reader runs in flexible mode.
5. **Thousands separators and quoting.** `"-1,250.00"` arrives quoted; so do
   descriptions containing commas (`"ACME HVAC SERVICES, INC"`).
6. **Money is never a float.** Amounts are parsed to integer cents by string
   manipulation and emitted as a string decimal. `1.005` is rejected as not
   money rather than rounded.
7. **A present-but-malformed transaction date is an error, never a silent
   fallback to the posting date.** Dating a deduction to the wrong day — or the
   wrong tax year — is the one failure this tool must not ship. The posting
   date is used only when the transaction date is genuinely absent.
8. **Spreadsheet round-trips are common.** A file that has been opened and
   re-saved may carry ISO dates, `M/D/YY` dates, long-form
   `"May 15, 2026"` dates, `($142.10)` accounting negatives, `$` symbols, or a
   trailing-minus `24.99-`. All are accepted; two-digit years resolve to the
   21st century.
9. **HTML entities** (`&amp;`) appear in descriptions that have been through a
   web view. They are decoded. Nothing else about the merchant string is
   touched — no `SQ *` stripping, no store-number removal. That normalization
   is lossy and belongs downstream, if anywhere.
10. **Blank cells read as absent**, not as empty strings, so `memo` and
    `category` are omitted from the DTO rather than emitted as `""`.

### Output shapes

- `import csv`, `import scan --transactions` → `transaction-list/v1`
  (`Paged<Transaction>`).
- `import scan` → `chase-csv-file-list/v1`.
- `import inspect` → `chase-csv-file/v1`.

`Transaction`'s first four fields — `date`, `amount`, `description`, `kind` —
are deliberately `pk_cli_utility::Transaction`'s fields, in its order, so a
consumer that already reads `transaction-list/v1` from a utility CLI reads
Chase output unchanged. The Chase-specific columns follow and can be ignored.
The crate is not depended on: a card export is not a utility portal, and
PROFILES.md's bar for minting a new profile (≥2 CLIs, a consumer paying for the
variance today) is not met by one CLI.
