# Phase 2 summary — Wire `smelt migrate` (plan-only)

**Shipped:**
- `smelt migrate <model>` CLI verb (`crates/smelt-cli/src/commands/migrate.rs`, wired via
  `MigrateArgs`/`Commands::Migrate` in `main.rs`) — derives a `DefinitionDiff` between the
  model's last-deployed SQL and its current SQL, classifies it through
  `smelt_logical::backbuild`, and prints a per-column-group plan (verdict, technique, statement
  count, refusals) plus a stable plan hash. Executes nothing.
- `DeployedSchema.model_sql: Option<String>` (`crates/smelt-state/src/schema_tracking.rs`, back-compat
  via `#[serde(default)]`), now persisted by both `save_deployed_schema` and the ALTER-TABLE
  migration path in `crates/smelt-runtime/src/schema_evolution.rs` — this is the "definition the
  stored table was last built under" `smelt migrate` diffs against.
- `crates/smelt-logical/src/backbuild/plan.rs`: `MigrationVerdict`, `ColumnGroupPlan`,
  `MigrationPlan`, `derive_migration_plan`, `plan_hash` (SHA-256 over the plan's derived shape
  plus the input facts it was derived from — table, after_sql, row_identity, not_null_columns,
  added_column_types, sources). No new SQL statement authoring anywhere in this module.
- Tests: 8 new in `crates/smelt-logical/tests/migration_plan.rs`, 4 new in
  `crates/smelt-cli/tests/migrate_plan.rs` (real DuckDB-backed CLI subprocess tests), 3 new unit
  tests across `schema_tracking.rs`/`schema_evolution.rs`.
- `docs/specs/definition_deltas.md` §Known Divergences narrowed (two bullets reworded — the
  synthesis layer is no longer "unwired"; what remains is `--apply`/`--json`/run-time refusal).

**Decisions:**
- Verdict folding: `SkeletonChange` covers both genuine grain/skeleton refusals *and* any atom
  this phase's classifier admits zero options for (including `Unclassified`) — "no in-place
  technique admitted, full refresh is the only honest route" is one semantic regardless of cause.
  `BackfillInPlace` vs `Rederive` is decided by whether any admitted option's `reads_upstream` is
  true. Whole-plan `MigrationPlan::verdict()` picks the worst group in priority
  `SkeletonChange > Rederive > BackfillInPlace`, `Eclipsed` only when there are zero groups.
- `plan_hash(plan, inputs)` takes both — the plan alone doesn't retain `BackbuildInputs`, and the
  phase-1 decision explicitly scopes the hash over input facts (sources, row_identity, etc.), not
  just the derived plan shape.
- `sources: BTreeMap<String, SourceRef>` in the CLI's `BackbuildInputs` construction is
  deliberately best-effort: keyed by each `smelt.ref()`'s leaf name (not proven FROM-tree-alias-exact),
  legacy `sources.yml` entries get `unique_key: None`/empty `not_null_columns` (that format has no
  such facts), upstream-model refs pull `unique_key` from `ModelMetadata` and `not_null_columns`
  from that upstream's own deployed schema when one exists. Fail-closed: an unresolved or
  under-specified source only costs admitted techniques, never a wrong admission.
- Also populated `model_sql` in `check_and_migrate`'s ALTER-TABLE arm (not just
  `save_deployed_schema`) even though only the latter was named in the plan's Task 1 — otherwise
  every ALTER-TABLE-only schema migration would silently drop the recorded definition and
  `smelt migrate` would wrongly report "no recorded definition" on the very next invocation.

**For the next planner:**
- Phase 3 (approval store + `--apply` + `--json`) can build directly on `MigrationPlan`/`plan_hash`
  as shipped — no rework anticipated.
- `smelt run`'s run-time detection refusal (spec §"Detection": refuse to fold data deltas over a
  pending non-eclipsed definition delta) is still unimplemented — noted in the spec's Known
  Divergences update, not a Success-criteria item for this phase, flagged for phase 3 or the
  phase-9 validate sweep.
- The `sources` map's leaf-name keying (rather than true FROM-tree-alias binding) is a real
  simplification worth revisiting once a self-join or aliased-multiple-reference model needs
  `smelt migrate` to admit B3/B4/D2-class techniques correctly — out of this phase's scope but
  worth a follow-up test/fixture in a later phase if it turns out to matter for a real model.

**Gates (all run this session, actual output confirmed, not the delegated agent's claim alone):**
- `bash .claude/scripts/verify-phase.sh` → `VERIFY: ALL GREEN`
- `cargo test -p smelt-logical --test migration_plan` → 8 passed
- `cargo test -p smelt-cli --test migrate_plan` → 4 passed
- `cargo test -p smelt-runtime --test statement_parity` → 23 passed
- `cargo test -p smelt-logical --test walk_coverage` → 4 passed
