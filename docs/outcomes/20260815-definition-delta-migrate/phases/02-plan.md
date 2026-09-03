# Phase 2 — Wire `smelt migrate` (plan-only)

## Objective

Make the backbuild synthesis layer reachable from the CLI: `smelt migrate <model>` derives a
definition diff between the definition the stored table was last built under and the model's
current SQL, classifies it through `smelt_logical::backbuild`, and prints a per-column-group
plan (verdict + candidate techniques + refusals + plan hash) while executing nothing. Advances
success criterion 1, and lays the plan-hash derivation criterion 2's approval store will persist.

## Spec delta

`docs/specs/definition_deltas.md` §"Known Divergences / Open Questions" — narrow, do not delete,
the first two bullets:

- "The definition-delta synthesis layer is unwired" → the layer is reached by `smelt migrate`'s
  plan path; what remains unwired is execution (`--apply`, phase 3) and the run-time detection
  refusal (`smelt run` still folds data deltas over a pending non-eclipsed delta).
- "`smelt migrate` does not exist" → `smelt migrate <model>` exists as a plan-only verb; `--apply`
  and `--json` do not yet, and the ranged-rebuild verb is still named `smelt backbuild`
  (phase 4). Keep the "column additions only" description of the live maintenance-driver path.

No §Surface change: this phase implements surface the spec already states.

## Tests

Red-green, in this order.

**`crates/smelt-state/src/schema_tracking.rs` (unit)**
- `deployed_schema_roundtrips_model_sql` — the persisted snapshot carries the definition SQL text.
- `deployed_schema_without_model_sql_still_loads` — a pre-existing on-disk snapshot (no
  `model_sql` key) deserialises, field `None`; `#[serde(default)]` back-compat.

**`crates/smelt-runtime/src/schema_evolution.rs` (unit)**
- `save_deployed_schema_records_the_definition_sql` — the `model_sql` argument it already takes is
  persisted, not only hashed.

**`crates/smelt-logical/tests/migration_plan.rs` (new)**
- `formatting_only_change_is_eclipsed` — a whitespace/alias-order-only edit yields one plan with
  no groups and verdict `Eclipsed`; the plan reports "nothing to do".
- `self_derived_column_add_is_backfill_in_place` — `net_amount = amount - discount` over stored
  columns groups as one `BackfillInPlace` verdict carrying `SelfDerivedColumnAdd`.
- `upstream_pull_through_is_rederive` — an added column reading a declared-unique upstream key
  groups as `Rederive`.
- `group_by_change_is_skeleton_change` — a changed `GROUP BY` yields verdict `SkeletonChange`
  with the full-refresh baseline as the only route, never an in-place technique.
- `unclassifiable_change_surfaces_its_refusal` — an opaque diff yields a named refusal, never an
  empty plan (fail-loud).
- `plan_hash_is_stable_across_derivations` — deriving twice from identical inputs hashes equal.
- `plan_hash_changes_when_an_input_fact_changes` — flipping a `SourceRef::unique_key` changes the
  hash (per phase-1 decision: input facts are in scope).
- `plan_hash_ignores_region_enumeration` — a plan differing only in the regions listed hashes
  equal (phase-1 decision: enumeration is resolved at apply time).

**`crates/smelt-cli/tests/migrate_plan.rs` (new, `#![cfg(feature = "duckdb")]`)**
- `migrate_prints_per_group_verdict_and_technique` — build a model, edit it to add a self-derived
  column, `smelt migrate` prints the group, verdict, technique and a plan hash.
- `migrate_on_unchanged_definition_prints_nothing_to_do` — exit 0, no groups.
- `migrate_without_a_recorded_definition_refuses_loudly` — a never-built model errors naming the
  missing recorded definition, rather than diffing against nothing.
- `migrate_executes_nothing` — after `smelt migrate` the stored table's contents and schema are
  byte-identical to before.

## Tasks

1. Add `#[serde(default)] pub model_sql: Option<String>` to `smelt_state::schema_tracking::DeployedSchema`;
   populate it in `smelt_runtime::schema_evolution::save_deployed_schema` from the `model_sql` it
   already receives. Fix the construction sites (`file_store.rs` tests, `history.rs`).
2. New `crates/smelt-logical/src/backbuild/plan.rs`: `MigrationVerdict`
   (`Eclipsed` | `BackfillInPlace` | `Rederive` | `SkeletonChange`), `ColumnGroupPlan` (columns,
   verdict, candidate options with technique/write scope/statement count/`rerun_safe`, named
   refusals), `MigrationPlan` (model, table, groups, full-refresh baseline, plan hash input).
3. `pub fn derive_migration_plan(diff: &DefinitionDiff, inputs: &BackbuildInputs) -> MigrationPlan`
   — pure, folds `derive_backbuild_options`' atoms into groups and maps each atom's admitted
   `Technique`s to its verdict. No new statement authoring: every SQL string comes from a
   `BackbuildOption` (statement single-ownership).
4. `pub fn plan_hash(plan: &MigrationPlan) -> String` — stable hash over verdicts, techniques,
   statements and the input facts (`table`, `after_sql`, `row_identity`, `not_null_columns`,
   `added_column_types`, `sources`), excluding region enumeration. Re-export both from
   `backbuild::mod`.
5. New `crates/smelt-cli/src/commands/migrate.rs`: resolve project root + target (mirroring
   `commands/backbuild.rs` steps 1–3), load the recorded definition via `FileStore::load_schema`,
   read the model's current SQL from discovery, build `BackbuildInputs` (table from the model's
   relation name; `not_null_columns` from the snapshot's `nullable` flags; `added_column_types`
   from the model's inferred projection; `sources` from `SourcesConfig` + upstream model
   metadata, absent facts left empty = fail-closed), derive and render the plan.
6. Render function in the CLI module: one line per group (columns, verdict, technique, cost
   posture), refusals listed explicitly, trailing `plan hash: <hash>   approve and execute with:
   smelt migrate <model> --apply` — matching the spec's worked example shape. `--apply` is not
   yet a flag; the hint names phase 3's surface.
7. Wire `Commands::Migrate(MigrateArgs)` into `crates/smelt-cli/src/main.rs` and
   `commands/mod.rs`; no execution path, no backend connection required beyond schema load.
8. Land the §Known Divergences narrowing from the Spec delta above.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test migration_plan --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test migrate_plan --quiet 2>&1 | tail -20`
- `cargo test -p smelt-runtime --test statement_parity --quiet 2>&1 | tail -20`
  (the plan must not author statements)
- `cargo test -p smelt-logical --test walk_coverage --quiet 2>&1 | tail -20`

## Commit message

`feat(cli): derive and print the definition-delta migration plan via smelt migrate`
