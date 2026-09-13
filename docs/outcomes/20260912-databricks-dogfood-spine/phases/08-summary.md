# Phase 8 summary — dual-target parity: DuckDB vs Databricks

**Status: blocked.** Tasks 1-8 done and committed; task 9 (live sweep) ran and found two
divergences, one root-caused and registered, one not; task 10 (root-cause everything) is
incomplete for the second divergence.

## Shipped

- `crates/smelt-cli/tests/parity_support/` (renamed from `bq_parity_support/`): shared
  comparator, landing seam, relation discovery/exclusion, and now a shared `Checkpoint`/
  `ParityManifest` and a new `DivergenceBound::UnorderedColumnDivergence` variant (licenses
  one named column to differ with no enforced direction, unlike `MonotoneDivergence`).
- `BIGQUERY_EXCLUDED_MODELS` / `DATABRICKS_EXCLUDED_MODELS` (empty) split out of the old
  single `EXCLUDED_MODELS`; `load_bigquery_snapshot` renamed `load_exported_snapshot` with
  its doc comment generalised to the export-encoding-contract framing.
- `github_activity_dual_target.rs`'s Databricks sweep: `SIDES_DBX`,
  `DBX_DIVERGENCE_REGISTRY` (one entry), `check_databricks_agree`, six offline tests, and
  the live test `duckdb_and_databricks_agree_on_every_model` (gated,
  `SMELT_DBX_DOGFOOD_LIVE=1`).
- `scripts/dbx_dogfood_export.py` — one Databricks Connect session, `information_schema`
  relation discovery with the same exclusion rules, per-relation NDJSON in the shared
  export encoding.
- `scripts/dbx-dogfood-parity.sh` — `duck`/`dbx-snapshot`/`manifest`/`report` stages, no
  destructive stage (the live Databricks state is the thing under test).
- Live: backfilled `gold.events_enriched`'s `[2026-08-07, 2026-08-10)` coverage gap
  (`07-summary.md`'s flagged hole) to a clean, contiguous `[2026-08-05, 2026-08-13)` for
  every window-addressed model.

## Decisions

- Generalisation route: rename + shared struct + split exclusion constant, per the
  plan-time decision log entry — see `outcome.md`.
- Registered `gold_events_enriched.current_repo_name`'s divergence (7 rows) as a new
  `UnorderedColumnDivergence` bound. Root cause: phase 7b's `EnrichmentKeyed` →
  `DeleteInsert` downgrade (no `MergeLedger` on Spark/Delta) sacrifices the unwindowed
  run-level heal DuckDB's `ColumnScopedMerge` cell performs — exactly the trade-off 7b's
  own decision log named as deferred to `databricks-correctness`.
- Deliberately did NOT commit `08-parity.json` or add `dbx_registry_entries_are_all_live`
  this phase — a second, unresolved divergence in `silver_actor_naming` would make that
  ratchet permanently red. Full details in `outcome.md`'s decision log and `## Blocked`.

## For the next planner

- **`silver_actor_naming` has up to 7x row duplication on Databricks, not present on
  DuckDB** — 520 extra rows, 26,177 distinct vs DuckDB's 25,700 over an identical source.
  The succession-patch technique's write path
  (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`) is the suspect;
  not read in this phase. See `outcome.md` `## Blocked` for three candidate resolution
  routes.
- This may share a root cause with the outcome's own 2026-09-13 research note on Catalog
  Commits / cross-table transactions on Delta (`MergeLedger`'s "permanent Spark absence"
  premise) — worth triaging together rather than separately, since both are about the
  maintenance layer's write primitives lacking a transactional guarantee on Spark/Delta
  that Databricks specifically may now offer.
- Once resolved: re-run `dbx-snapshot`/`manifest`/the live test, commit the resulting
  `08-parity.json`, and restore `dbx_registry_entries_are_all_live` (the code shape is
  described in this phase's own git history — it was written, tested green, and removed
  only because the artifact it depends on wasn't yet clean).

## Gates

- `cargo test -p smelt-cli --test github_activity_dual_target` — 23/23 pass.
- `cargo test -p smelt-cli --test github_activity_bq_oracle` — 14/14 pass (rename did not
  disturb the BigQuery equivalence sweep).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, full `cargo test`, `example_diagnostics`).
- Live: `SMELT_DBX_DOGFOOD_LIVE=1 cargo test -p smelt-cli --test github_activity_dual_target
  duckdb_and_databricks_agree_on_every_model -- --nocapture` — ran, found the two
  divergences above (not re-run after registering the first, since the second still fails
  it; not part of the standing gate).
