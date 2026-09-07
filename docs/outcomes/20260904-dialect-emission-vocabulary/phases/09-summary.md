# Phase 9 summary — close the DATE_ADD/DATE_SUB type-leg family

**Shipped:**
- `DATE_ADD`/`DATE_SUB` are ordinary `SyntaxForm::Call` registry rows with `SqlFunction::DateAdd`/
  `DateSub` variants (`crates/smelt-types/src/functions.rs`), on `REGISTRY_MIGRATED`
  (`crates/smelt-db/src/type_inference/function_call.rs`) — both now infer `Timestamp` via the
  registry-first path instead of bailing at `SqlFunction::from_name` before the registry runs.
- DuckDB stays `Native` for `DATE_ADD`; `DATE_SUB` keeps `Emission::Template("{0} - {1}")`.
  Spark gained measured casts: `DATE_ADD` → `Template("CAST({0} + {1} AS TIMESTAMP)")`,
  `DATE_SUB` → `Template("CAST({0} - {1} AS TIMESTAMP)")` — the bare infix reports `DATE` on
  Spark, not smelt's declared `Timestamp`; live-measured via a freshly bootstrapped
  `.smelt-spark-venv` against Spark 4.0.0 Connect (`typeof(...)` = `timestamp` for both).
- `validate_conditional` now validates every `SettledEmission::Template` arm verdict through
  `validate_template` (new `ConditionalError::InvalidTemplateArm`, threading the arm index),
  closing the criterion-2 hole phase 8 surfaced — `validate_conditional` gained a `position`
  parameter to support this.
- `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_duckdb` 6→4, `dialect_gaps_spark` 6→4 —
  criterion 5 now lands on both live engines, leaving only the four `#175`/`#176` rows
  (`FIRST`/`LAST`/`EXPLODE`/`UNNEST`) per dialect. Ledger rows deleted: 2 DuckDB `type_gap`,
  1 Spark `gap`, 1 Spark `type_gap`, 1 stale Spark `divergent` (the cast closes the value gap too).
- Spec deltas: `architecture.md` item 14's Consistency-gate parenthetical no longer names
  `DATE_ADD`/`DATE_SUB` as dedicated-syntax exemptions; `multi_backend.md` §"Template emission"
  states a `Conditional` arm's `Template` verdict is validated by the same rules as a top-level
  template row.
- New/updated tests: `registry_coverage::date_add_and_date_sub_are_ordinary_calls`,
  `::a_conditional_arm_template_is_validated`; `registry_inference::date_add_infers_timestamp`,
  `::date_sub_infers_timestamp`; `template_emission::date_add_prints_the_spark_form`,
  `::date_sub_spark_form_matches_smelt_return_type`; updated the pre-existing
  `operand_conditional::date_sub_prints_the_spark_form` pin to the new cast form; removed
  `DATE_ADD`/`DATE_SUB` from `dedicated_syntax_entries_are_not_call_form`'s list.

**Decisions:**
- Made the pair genuinely callable rather than excluding non-`Call` syntax forms from the probe
  axis (plan's pre-decided call, confirmed: nothing in production consumed the `Special`
  classification — `binary.rs` types the infix form itself).
- Bootstrapped `.smelt-spark-venv` (gitignored, per `scripts/README-spark.md`'s one-time setup)
  since it did not exist in this worktree; measured both Spark casts live before registering
  them, rather than trusting the plan's a-priori guess (which turned out correct).

**For the next planner:**
- Phase 10 (docs: architecture.md/CLAUDE.md invariant text, docs-site diagnostics page, ROADMAP,
  tracking-issue updates for the BigQuery sweep) is next and unaffected by this phase's scope.
- Pre-existing, unrelated flake confirmed again: `smelt-core --test baseline`'s
  `materialize_tests::checkout_scratch_is_deleted_when_materialization_fails` fails under the
  full parallel `cargo test` run (temp-file race across concurrently running tests sharing
  `/tmp/smelt-baseline-*`) but passes 21/21 with `--test-threads=1`. Not touched by this phase's
  diff (confirmed via `git diff --stat -- crates/smelt-core/`). Worth a dedicated outcome/fix,
  not scoped here.
- `.smelt-spark-venv` now exists in this worktree only — other worktrees will need the same
  one-time bootstrap (`scripts/README-spark.md` §"One-time client setup") before their next
  live-Spark phase.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/example_diagnostics PASS; `cargo test`
  (workspace) FAILED only on the pre-existing unrelated flake above (confirmed via isolated
  single-threaded rerun).
- `cargo test -p smelt-db --test integration -- registry_consistency registry_inference` — PASS
  (11/11).
- `cargo test -p smelt-types --test registry_coverage` — PASS (106/106).
- `cargo test -p smelt-dialect --test template_emission --test emission_ownership --test operand_conditional --test unsupported_emission` — PASS (11+8+6+18).
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity` — PASS (16+4+1).
- `cargo test -p smelt-db --test type_property_tests` — PASS (89/89).
- `SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) cargo test -p smelt-db --test dialect_audit` — PASS (61/61), live Spark 4.0.0 Connect.
- `git diff .claude/dialect-gaps-baseline.txt docs/reference/dialect-coverage.md` — both intended.
