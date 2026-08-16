# Handoff — BigQuery as a first-class backend

**Date:** 2026-08-16
**Worktree:** `/home/andrew/smelt-sql/.claude/worktrees/bigquery`
**Branch:** `bigquery-backend-research` (clean descendant of `main`; no PR yet)

## Where we are

Phase 0 (provisioning) is complete and the `multi_backend.md` spec diff has landed.
Nothing is implemented in Rust yet — no `BackendType::BigQuery`, no dialect variant, no
backend crate.

**Read first:** `docs/research/20260816-bigquery-backend.md`. It holds the decisions, the
rejected alternatives and why, the phase order, and the provisioned environment. This
handoff does not restate it.

## The decisions that constrain everything downstream

1. **Client layer is a PyO3 → Python adapter**, mirroring `python/smelt/spark_adapter.py`,
   using `google-cloud-bigquery` and Arrow. Not a Rust crate: the available ones return JSON
   rows rather than Arrow, and the workspace pins `arrow = 58` to DuckDB's version.
2. **Verification is local-gated with no CI tier.** BigQuery tests skip green when
   `SMELT_BQ_PROJECT` is unset. Recorded as a Known Divergence, not left implicit.
3. **Ordering is walking-skeleton-first.** Get one model materialising before any breadth
   work, because the unknown is the loop (auth, latency, quota), not GoogleSQL.
4. **Credentials are non-ambient — do not "simplify" this.** No application-default
   credentials at any point: ADC carries Andrew's entire Google Cloud identity across every
   project, which is a far worse blast radius than a dataset-scoped service account. The
   adapter must authenticate from `SMELT_BQ_ACCESS_TOKEN` explicitly and never fall back.

## Provisioned environment (do not re-create)

Project `smelt-bq-test-20260816`; dataset `smelt_test` (US, 24h default table expiration);
control dataset `smelt_test_notgranted` (exists so the least-privilege check has something
real to be refused from); service account `smelt-bq-test@smelt-bq-test-20260816.iam.gserviceaccount.com`
holding project-scoped `bigquery.jobUser` plus `WRITER` on `smelt_test` only. Isolated
gcloud config at `~/.config/gcloud-smelt-bq`; SA key gpg-encrypted at rest.

Scripts (`scripts/`): `bigquery-login.sh` → `bigquery-provision.sh` → `bigquery-key.sh` for
setup; `bigquery-auth.sh` + `bigquery-env.sh` per session; `bigquery-verify.sh` to prove the
chain. `bigquery-setup.sh` is the single-command path for a fresh machine.

**Start a session with:** `bash scripts/bigquery-auth.sh` (prompts for the passphrase) then
`source scripts/bigquery-env.sh`.

## Working constraints that cost time to rediscover

- `.claude/settings.json` **denies** `gcloud`, `bq`, and the BigQuery scripts, so an agent
  cannot reach GCP. Relax only with Andrew's explicit say-so, and restore immediately after.
- Interactive flows — gcloud OAuth, the gpg passphrase — need a **real terminal**. They fail
  under both the Bash tool and the `!` prefix, which have no TTY.
- `bq` shells out to `gcloud`, so the SDK bin dir must be on `PATH`; absolute paths alone fail.
- The worktree guard rejects compound shell commands (pipes, env-var prefixes, `$(…)` plus
  redirects). Put anything non-trivial in a script and run the script.

## Measured, and still unknown

Round-trip cost, measured 2026-08-16: `SELECT 1` **745 ms**, `CREATE OR REPLACE TABLE`
**1,945 ms** — three orders of magnitude above in-process DuckDB. This is the constraint that
shapes Phase 7.

Still unknown, and **Phase 1's first job**: the per-table DML rate limit. `maintenance_conformance`
applies repeated DML to one table, which is exactly the shape BigQuery throttles. Measure it with
a tight DML loop before designing around a number. If it binds, allocate a fresh target table per
generative case rather than cutting cases.

## Next steps, in order

1. **`/smelt:plan`** off the `multi_backend.md` spec diff (commit `<this branch>`), producing
   `docs/plans/2026MMDD-bigquery-backend.md`.
2. **Phase 1 walking skeleton** — `BackendType::BigQuery`, `Target` fields (`project`,
   `dataset`, `location`), `SqlDialect::BigQuery`, a best-effort capability constructor,
   `python/smelt/bigquery_adapter.py`, `crates/smelt-backend-bigquery`, the `create_backend`
   arm behind a `bigquery` feature, and `bigquery_smoke.rs` mirroring `spark_smoke.rs`.
   Done when one model materialises **and** latency plus the DML rate limit are measured.
3. **Phase 2 dialect** — fill the capability struct empirically, one flag at a time against
   the live warehouse, then populate the spec's matrix column in the same commit.

Do not populate the capability matrix from documentation. The spec calls that table the honest
matrix and the conformance test asserts it against the code constructors; guessed values would
be unverified assertions in a normative document.

## Loose ends

- **The budget alert is unprovisioned.** `gcloud billing budgets` requires ADC, which this
  design refuses. Manual console step: US$5/month on `smelt-bq-test-20260816`. Note a GCP
  budget alerts rather than hard-stops.
- **The fixed least-privilege check has not been re-run.** The original version failed with
  "dataset not found" — the same error an owner would get — so it verified nothing. It now
  targets the control dataset and asserts on `Access Denied` specifically. Re-run
  `bash scripts/bigquery-provision.sh smelt-bq-test-20260816 017727-8AD4F3-C0182A` then
  `bash scripts/bigquery-verify.sh` and confirm it reports **DENIED** before building on the
  grant.
