# Phase 9a — the Databricks equivalence-oracle harness, offline

## Objective

Build everything criterion 8 needs *except* the live runs: a `databricks_oracle` target that
writes to `smelt_dogfood_oracle` while reading the shared source tables, an equivalence sweep
over the same `parity_support` landing seam phase 8 generalised, and a driver script whose
stages the sweep's manifest matches. Every claim in this phase is provable with no workspace,
so phase 9b spends its one-hour token purely on running windows.

## Spec delta

None. Adding a target to the example project and a test suite over it changes no user-visible
feature behaviour; `docs/specs/sources.md` §"Target-aware `name:` override" and
`docs/specs/incremental_models.md` §"The equivalence invariant" already describe every
mechanism used here.

## Tests

All in `crates/smelt-cli/tests/github_activity_dbx_oracle.rs` unless noted.

- `the_oracle_target_resolves_sources_to_the_shared_tables` — through the real
  `SourceInfo::db_name_for_target`, `databricks_oracle` resolves both raw sources to
  `smelt_dogfood.github_events{,_arrival}`, not to anything under `smelt_dogfood_oracle`. This
  is the anti-vacuity gate: without it the oracle refreshes over a table nothing creates.
- `the_oracle_target_writes_only_to_the_oracle_schema` — its `schema`/`catalog` are
  `smelt_dogfood_oracle`/`workspace`, so the oracle cannot overwrite the state it judges.
- `adding_the_oracle_target_does_not_move_the_default` — `default_target` is still `dev`.
- `an_unregistered_equivalence_violation_fails` — synthesised pair of DuckDB databases; the
  sweep's verdict path names the relation rather than passing.
- `the_equivalence_sweep_fails_closed_on_an_empty_registry` — an empty registry must still
  fail a differing pair (no "nothing registered, therefore fine" path).
- `a_relation_missing_from_the_oracle_side_is_a_coverage_failure` — relation-set totality.
- `the_databricks_oracle_exempts_no_relation` — the exemption list is empty, and the sweep's
  per-checkpoint source-coverage assertion exists, i.e. the BigQuery exemption machinery is
  replaced by a measurement rather than dropped.
- `a_checkpoint_whose_source_ran_ahead_of_its_window_fails` — over a synthetic manifest, a
  checkpoint recording more source days than its window number fails; this is the assertion
  that makes "the source holds only the inputs seen so far" measured.
- `the_oracle_driver_declares_the_checkpoint_schedule_the_sweep_expects` — the default
  `START_DATE`/checkpoint list parsed out of `scripts/dbx-dogfood-oracle.sh` matches the
  constants the test file compares against, so script and suite cannot drift.
- `the_equivalence_report_covers_every_model_at_every_checkpoint` — reads the committed report
  when present; until 9b commits one it prints a loud skip. 9b flips it to a hard gate.
- Regression, run not written: `github_activity_bq_oracle` and `github_activity_dual_target`
  stay green across the `parity_support` extraction.

## Tasks

1. Add the `databricks_oracle` target to `examples/github_activity/smelt.yml` (type
   `databricks`, `catalog: workspace`, `schema: smelt_dogfood_oracle`, same `${SMELT_DBX_*}`
   env references as `databricks`), with a comment naming criterion 8 and why `target: dev`
   keeps it off the no-`--target` default.
2. Add `databricks_oracle:` entries to both `models/sources/raw/github_events{,_arrival}.yml`
   `name:` maps, pointing at the shared `smelt_dogfood.*` tables, with the same rationale
   comment the `bigquery_oracle:` entries carry.
3. Extract the machinery `github_activity_bq_oracle.rs` and the new file would otherwise
   duplicate verbatim — the equivalence manifest/checkpoint shape, the `check_equivalence`
   wrapper over `check_agreement_against`, and the synth-db/violating-pair helpers the negative
   controls drive — into `crates/smelt-cli/tests/parity_support/`. Each target's file keeps its
   own registry, report path and coverage notes. No behaviour change to the BigQuery suite.
4. Write `github_activity_dbx_oracle.rs`: module doc stating what the suite is and is not (one
   engine against its own full refresh, not a second dual-target sweep) and why no relation is
   exempt on this target, an empty `DBX_EQUIVALENCE_DIVERGENCE_REGISTRY` with a high bar
   documented for adding to it, the tests above, and the live sweep
   `databricks_incremental_matches_its_oracle_at_every_window` — gated on
   `SMELT_DBX_DOGFOOD_LIVE=1`, and under that flag **failing** rather than skipping when the
   manifest is absent.
5. Write `scripts/dbx-dogfood-oracle.sh` (modelled on `dbx-dogfood-parity.sh`, shellcheck
   clean) with stages: `duck-types` (delegates to the parity script's `duck` stage for the
   type-reference databases at windows 9-11), `window <day>` (loader day, then
   `smelt run --target databricks` for that window), `oracle` (`smelt run --full-refresh
   --target databricks_oracle`), `snapshot` (both schemas exported via
   `SMELT_DBX_SCHEMA=… scripts/dbx_dogfood_export.py`), `manifest`, `report`. Read-only on
   `smelt_dogfood` except through `smelt run --target databricks` itself.
6. Allow-list `scripts/dbx-dogfood-oracle.sh` and `scripts/dbx-dogfood-parity.sh` in
   `.claude/settings.json` alongside the other `dbx-*` wrappers.
7. Write `phases/09a-summary.md` naming exactly what 9b must run and which test it flips to a
   hard gate.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt, clippy both feature sets, shellcheck, full
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test github_activity_dbx_oracle`
- `cargo test -p smelt-cli --test github_activity_bq_oracle` and
  `cargo test -p smelt-cli --test github_activity_dual_target` — the extraction disturbs
  neither existing suite.
- No live workspace is touched; nothing in this phase may require one.

## Commit message

`outcome(databricks-dogfood-spine): phase 9a builds the Databricks equivalence oracle harness offline`
