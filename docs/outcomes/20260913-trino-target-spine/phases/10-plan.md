# Phase 10 plan — CI runs the Trino tier, gated like Spark's

## Objective

Land criterion 8: a `compat.yml` job that stands up the pinned Trino/Iceberg/MinIO tier and
runs every Trino live suite, gated exactly the way `spark-parity` is (schedule ∨
`run-docker-tests` label ∨ a `changes` path filter), plus the two-sided skip proof — unset
`SMELT_TRINO_URL` must *skip* everywhere (never fail, never vacuously pass), and with the tier
up in CI a leg that skips must *fail the job*. Phase 9 already proved the skip half at the
`trino_env()` level; this phase proves it across every Trino-gated test file and adds the
CI-side "nothing skipped" guard.

## Spec delta

None. No user-visible feature behaviour changes — this is CI wiring plus test-side gates.
(`scripts/README-trino.md` gains a short "In CI" paragraph as documentation, not spec.)

## Tests

New file `crates/smelt-cli/tests/trino_ci_wiring.rs` — text gates over `.github/workflows/compat.yml`
and the Trino test sources, in the shape of `crates/smelt-cli/tests/property_diff_ci_docs.rs`
(pure file assertions, no Docker, no network):

1. `compat_workflow_has_a_trino_job_gated_like_spark` — a `trino-integration:` job exists, `needs: changes`,
   and its `if:` carries all four of `!cancelled()`, `github.event_name == 'schedule'`,
   `'run-docker-tests'`, and `needs.changes.outputs.trino == 'true'`. Fails against a job that
   silently runs on every PR, or one that a failing `changes` job would skip.
2. `changes_job_declares_a_trino_filter_covering_every_trino_path` — the `changes` job outputs `trino`,
   the `trino:` filter block exists, and every path the tier actually lives at
   (`scripts/trino-up.sh`, `scripts/trino-compose.yml`, `scripts/trino-catalog/iceberg.properties`,
   `crates/smelt-backend-trino/src/lib.rs`, `examples/trino_spine/smelt.yml`,
   `crates/smelt-cli/tests/trino_smoke.rs`) is matched by at least one glob in it. Paths are read
   off the filesystem, not restated, so a moved file fails the gate.
3. `the_trino_job_brings_the_tier_up_runs_the_live_suites_and_tears_it_down` — the job's steps run
   `scripts/trino-up.sh`, export the `SMELT_TRINO_*` vars, invoke `-p smelt-backend-trino` and each
   CLI Trino leg (`trino_smoke`, `seed_parity`, `materialization_parity`), and carry an
   `if: always()` `scripts/trino-down.sh` teardown.
4. `the_trino_job_fails_if_a_leg_skips_with_the_tier_up` — the job carries the no-skip guard
   (a `grep`-based check over the captured test output under `set -o pipefail`). This is the
   anti-vacuous-pass half that matters in CI, where the URL *is* set: a green job in which every
   leg skipped is indistinguishable from one that passed.
5. `every_trino_gated_test_file_skips_through_the_shared_env_gate` — census over the five files
   with live Trino legs (`smelt-backend-trino/tests/{statement_client,backend_live,capability_probes}.rs`,
   `smelt-cli/tests/{trino_smoke,seed_parity}.rs`): each reads `SMELT_TRINO_URL` via a helper that
   returns `Option`/`None` and prints a `Skipping …` line, and none supplies a fabricated default
   URL (`unwrap_or`/`unwrap_or_else` on the `SMELT_TRINO_URL` lookup). Fails against a new Trino
   suite added with an ad-hoc gate — the hole an unset `DUCKDB_LIB_DIR` opened.

## Tasks

1. Write `crates/smelt-cli/tests/trino_ci_wiring.rs` with tests 1–5; confirm all five fail red first.
2. Rename the `changes` job's display name to cover both backends, add a `trino:` filter and a
   `trino` output alongside `spark` (include `crates/smelt-backend-trino/**`, `crates/smelt-backends/**`,
   `crates/smelt-cli/tests/trino_*`, `crates/smelt-cli/tests/common/**`, `crates/smelt-runtime/**`,
   `crates/smelt-dialect/**`, `examples/trino_spine/**`, `scripts/trino-*`, `scripts/trino-catalog/**`,
   `scripts/README-trino.md`, `.github/workflows/compat.yml`).
3. Add the `trino-integration` job: checkout, setup-duckdb, mise, cargo cache keyed
   `cargo-trino-integration-…`, `bash scripts/trino-up.sh`, export `SMELT_TRINO_*` into `$GITHUB_ENV`
   (mirroring how `spark-parity` exports `SPARK_CONNECT_URL` — `trino-env.sh` is a `source`-only
   script and does not survive a step boundary), run the suites, teardown `if: always()`.
   No pyspark/venv/Ivy steps — Trino is pure Rust HTTP, which is the point.
4. Add the no-skip guard to the job's test steps (`set -o pipefail`, `tee` the output, fail with a
   `::error::` annotation if `Skipping` appears).
5. Add an "In CI" section to `scripts/README-trino.md`: which job runs the tier, its three triggers,
   and that a skipped leg is a CI failure there while it is the correct green outcome locally.
6. Run the gates; confirm tests 1–5 green.

## Verification

- `bash .claude/scripts/verify-phase.sh` (includes shellcheck, which covers any script edit).
- `cargo test -p smelt-cli --test trino_ci_wiring --test trino_tier_pins`.
- Live confirmation that the job's exact command list works and skips nothing:
  `bash scripts/trino-up.sh && source scripts/trino-env.sh`, then run each command the new job runs
  with `-- --nocapture`, asserting no `Skipping` line appears; `bash scripts/trino-down.sh`.
  If the tier cannot be reached, emit `<<PHASE_BLOCKED>>` — never record this leg as green unrun.

## Commit message

`ci(trino): run the Trino/Iceberg tier in compat.yml, gated like spark-parity`
