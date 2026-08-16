# Handoff — BigQuery as a first-class backend

**Date:** 2026-08-16
**Worktree:** `/home/andrew/smelt-sql/.claude/worktrees/bigquery`
**Branch:** `bigquery-backend-research` (clean descendant of `main`; no PR yet)

## Where we are

The walking skeleton is live and the capability matrix is populated. One model materialises
in a real BigQuery dataset through the ordinary `smelt run` pipeline, verified by
`cargo test -p smelt-cli --features bigquery --test bigquery_smoke`.

**Read first:** `docs/research/20260816-bigquery-backend.md` — decisions, rejected
alternatives, phase order, the provisioned environment, and the §"Measured against the live
warehouse" findings. This handoff does not restate it.

## The decisions that constrain everything downstream

1. **Client layer is a PyO3 → Python adapter** (`python/smelt/bigquery_adapter.py`), mirroring
   `spark_adapter.py`, using `google-cloud-bigquery` and Arrow.
2. **Verification is local-gated with no CI tier.** BigQuery tests skip green when
   `SMELT_BQ_PROJECT` is unset. Recorded as a Known Divergence.
3. **Credentials are non-ambient — do not "simplify" this.** No application-default
   credentials at any point. The adapter authenticates from `SMELT_BQ_ACCESS_TOKEN` explicitly
   and refuses to construct without one; `bigquery_smoke.rs` asserts that refusal, because it
   is a security property rather than an implementation detail.
4. **Capability values come from the warehouse, never from documentation.** Every cell in the
   matrix was established by running the statement the flag names (`scripts/bigquery-probe*.sh`).

## What is implemented

`BackendType::BigQuery`, `Target` `project`/`dataset`/`location`, `SqlDialect::BigQuery`,
`BackendCapabilities::bigquery()` (asserted by `capability_conformance`),
`crates/smelt-backend-bigquery`, the `create_backend` arm behind a `bigquery` feature, and
`MaintenanceDialect::BigQuery`.

Dialect work landed where the warehouse actually refused something:

- **Type names.** GoogleSQL rejects `VARCHAR`, `TEXT`, `DOUBLE`, `REAL`, `FLOAT`. The
  output-boundary cast wrap (`type_conformance.rs`) and the maintenance bootstrap DDL map them
  to `STRING`/`FLOAT64`/`BYTES`; everything else passes through.
- **`SHA256` returns `BYTES`**, so fingerprint emitters need a `TO_HEX` wrap no other dialect
  requires.
- **No `INSERT OVERWRITE`** — `insert_overwrite` lowers to the scoped `DELETE` + `INSERT`.
- **Schema-evolution DDL is unimplemented** — resolves to a full refresh naming the reason
  rather than emitting DDL BigQuery would reject.

## Two findings that change downstream sizing

- **The per-table modification quota binds.** Eight rapid `CREATE OR REPLACE TABLE` statements
  against *one* table name are refused (`Your table exceeded quota for table update
  operations`); the same rate across distinct tables is not. A generative conformance suite
  must allocate a fresh target table per case. An earlier reading that the limit did not bind
  came from a loop whose own round-trip latency kept it under the burst threshold — do not
  trust a rate measurement whose spacing is set by its own latency.
- **BigQuery supports native materialized views.** `supports_native_ivm` is nonetheless `false`,
  because `true` obliges smelt to emit a native maintained object and that path does not exist.
  It is the only flag whose value describes smelt rather than the engine.

## Working constraints that cost time to rediscover

- `.claude/settings.json` **denies** `gcloud`, `bq`, and the credential scripts, so an agent
  cannot reach GCP directly. It *can* run `scripts/bigquery-probe*.sh`, `bigquery-verify.sh`
  and `bigquery-test.sh`, which read the minted token off disk.
- Interactive flows — gcloud OAuth, the gpg passphrase — need a **real terminal**.
- **`bq` is unusable on this host** (bundled-Python pyOpenSSL mismatch:
  `module 'lib' has no attribute 'GEN_EMAIL'`). `gcloud` is fine. Everything talks to BigQuery
  over the REST API with `curl` + a token; do not reintroduce `bq`.
- **`bigquery-auth.sh` activates the service account**, which sets it as the gcloud config's
  active account. It now restores the previous account on exit — if that regresses, every
  human-identity operation silently runs as the service account and fails with
  `PERMISSION_DENIED`.
- The worktree guard rejects compound shell commands (pipes, `source`, `$(…)` plus redirects).
  Put anything non-trivial in a script and run the script.

## Session workflow

```
bash scripts/bigquery-venv.sh     # once: uv-based client venv (system python has no pip)
bash scripts/bigquery-auth.sh     # per session: prompts for the passphrase, 1h token
bash scripts/bigquery-test.sh     # runs the gated suites
```

## Next steps, in order

1. **`/smelt:plan`** for the remaining phases, now that the dialect surface has reported.
2. **Type oracle and divergences** — a BigQuery oracle beside `duckdb_oracle.rs` plus a
   divergence column. The surface is smaller than anticipated: the divergence is concentrated
   in type *spelling*, not semantics.
3. **Parity legs** — a third `TargetKind` in `crates/smelt-cli/tests/common/mod.rs`, gated on
   `SMELT_BQ_PROJECT`, which every parity suite picks up at once. This is the largest gap:
   the capability flags are verified as warehouse facts but not yet as parity outcomes, and
   `supports_pipe_syntax` / `supports_merge_not_matched_by_source` are `true` for the first
   time on any backend, so those printer paths are still unexercised.
4. **Incremental legs, schema evolution, generative conformance** — the last needs the
   fresh-table-per-case allocation above.

## Loose ends

- **The budget alert is unprovisioned.** `gcloud billing budgets` requires ADC, which this
  design refuses. Manual console step: US$5/month on `smelt-bq-test-20260816`. A GCP budget
  alerts rather than hard-stops.
- **One pre-existing test failure, unrelated to this work:**
  `cargo test -p smelt-logical --test contract_lattice_spec` wants a
  `### The contract, plan, and graph layer` heading in `docs/specs/incremental_models.md` that
  is absent at `HEAD`.
