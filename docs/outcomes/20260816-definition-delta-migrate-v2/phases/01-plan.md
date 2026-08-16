# Phase 1 — Wire `smelt migrate` (plan-only)

## Objective

Make the backbuild synthesis layer reachable from the CLI: `smelt migrate <model>` compares the
model's current compiled SQL against the definition the stored table was last maintained under,
runs diff → classify → plan derivation, and prints the per-group verdict/technique plan. It
executes nothing. Advances success criterion 1 and lays the plan data structure criterion 2's
plan hash will cover.

The enabling gap this phase closes: state records only a `definition_hash`, never the definition
text, so there is no "before" side to diff. The recorded definition SQL is persisted alongside
the deployed-schema snapshot — spec §Detection already promises smelt records "the definition it
last maintained the stored table under", so this is implementing the spec, not widening it.

## Spec delta

Made first, before code.

- `docs/specs/definition_deltas.md` §Detection — state where the recorded definition lives: the
  per-model deployed-schema snapshot (`.smelt/targets/<target>/schemas/<model>.json`) carries the
  definition SQL the table was last maintained under, alongside its hash.
- `docs/specs/definition_deltas.md` §Known Divergences — narrow two bullets, do not delete:
  - "The definition-delta synthesis layer is unwired" → now consumed by `smelt migrate`'s plan
    derivation; what remains unwired is execution (`--apply`).
  - "**`smelt migrate` does not exist**" → `smelt migrate <model>` exists and derives/prints the
    plan; `--apply`, `--json`, and the approval store do not (phase 2), the ranged-rebuild verb is
    still named `smelt backbuild` (phase 3), and the maintenance driver's narrower
    column-addition trigger still runs unchanged.
- `docs/specs/run_state.md` §"Fixed layout" / schemas-snapshot line — the snapshot records the
  deployed definition SQL; absent in pre-existing snapshots, which read back as "no recorded
  definition" (fail-closed).

## Tests

- `smelt-state` `schema_tracking::deployed_schema_round_trips_definition_sql` — save/load
  preserves the field.
- `smelt-state` `schema_tracking::legacy_snapshot_without_definition_sql_reads_empty` — a snapshot
  written before this field deserialises with an empty definition (never a panic, never a guess).
- `smelt-logical` `backbuild::plan::noop_diff_is_eclipsed_with_no_groups` — an A0 no-op derives an
  eclipsed plan with nothing to run.
- `smelt-logical` `backbuild::plan::technique_verdict_mapping_covers_every_technique` — exhaustive
  match: self-derived add/rewrite/rename → backfill in place; upstream/join/insert/delete/backfill
  techniques → re-derive; `FullRefresh` → the model-level baseline. A new `Technique` variant fails
  to compile rather than defaulting.
- `smelt-logical` `backbuild::plan::group_with_no_admissible_option_falls_back_to_full_refresh` —
  a refused atom is presented as full-refresh-baseline-only, carrying its named refusals.
- `smelt-logical` `backbuild::plan::skeleton_refusal_is_a_skeleton_change_verdict` — a skeleton
  refusal surfaces as the skeleton-change verdict, rebuild as the only route.
- `smelt-logical` `backbuild::plan::multiple_admissible_techniques_are_presented_as_candidates` —
  options-not-choices: both join-enrichment shapes survive into the group's candidate list.
- `smelt-runtime` `migrate::derive_migration_plan_reads_recorded_definition` — recorded definition
  + edited current SQL yields the expected group/verdict/technique.
- `smelt-runtime` `migrate::derive_migration_plan_without_recorded_definition_errors` — a model
  with no deployed-schema snapshot produces a named error, never an empty plan.
- `smelt-cli` `tests/migrate.rs::migrate_prints_backfill_in_place_plan_for_added_derived_column` —
  build a DuckDB project, add `net_amount AS amount - discount`, `smelt migrate <model>` prints the
  group, verdict, technique, and cost class; the deployed table is byte-identical afterwards
  (nothing executed).
- `smelt-cli` `tests/migrate.rs::migrate_presents_rebuild_for_skeleton_change` — a `GROUP BY`
  change prints the skeleton-change verdict and the full-refresh route, exit 0, nothing executed.
- `smelt-cli` `tests/migrate.rs::migrate_on_unchanged_model_reports_eclipsed` — no delta prints
  "eclipsed: nothing to do".

## Tasks

1. Apply the spec delta above.
2. `smelt-state`: add `definition_sql: String` (`#[serde(default)]`) to `DeployedSchema`; populate
   it at both save sites in `smelt-runtime/src/schema_evolution.rs` (`save_deployed_schema` and
   `check_and_migrate`'s post-migration save) from the `model_sql` each already receives.
3. New pure module `crates/smelt-logical/src/backbuild/plan.rs`: `MigrationPlan`,
   `ColumnGroupPlan`, `Verdict` (eclipsed / backfill in place / re-derive / skeleton change),
   `CostClass` (derived from `WriteScope` + `reads_upstream`), and
   `derive_migration_plan(&BackbuildOptions) -> MigrationPlan` — pure, exhaustive over `Technique`,
   carrying every candidate option and every named refusal, plus the always-present full-refresh
   baseline. Re-export from `backbuild/mod.rs`; update the module doc's "deliberately unwired" line.
4. New `crates/smelt-runtime/src/migrate.rs`: `derive_migration_plan_for_model(...)` assembles
   `BackbuildInputs` from real facts — table name, current compiled SQL as `after_sql`, recorded
   definition as the before side, `not_null_columns` and added-column types from the deployed
   schema / `infer_deployed_columns`, `row_identity` from the model's declared unique key, and one
   `SourceRef` per FROM-tree upstream (physical name, declared unique key, upstream deployed
   schema's non-nullable columns). Missing facts stay absent (fail-closed), never assumed. Calls
   `definition_diff` → `derive_backbuild_options` → `derive_migration_plan`. No backend writes.
5. CLI: `MigrateArgs { model, --project-dir, --target, --database }`, `Commands::Migrate`,
   `commands/migrate.rs` rendering the plan in the layout of `definition_deltas.md` §Overview's
   worked example (group, verdict, technique, cost class; refusals named; eclipsed short-circuit).
   The plan-hash line is phase 2 — do not print a fake hash.
6. Fail-loud sweep: unknown model, no recorded definition, and an opaque diff each print a named
   refusal and a non-zero exit where appropriate; no `unwrap`/`expect` added to production paths.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state --quiet 2>&1 | tail -20`
- `cargo test -p smelt-logical --quiet 2>&1 | tail -20` (includes `--test walk_coverage`)
- `cargo test -p smelt-cli --test migrate --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --quiet 2>&1 | tail -20`

## Commit message

`feat(migrate): derive and print the definition-delta migration plan (plan-only \`smelt migrate\`)`
