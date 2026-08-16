# Phase 1 summary — Wire `smelt migrate` (plan-only)

## Shipped

- `crates/smelt-logical/src/backbuild/plan.rs` — pure `derive_migration_plan(&BackbuildOptions) ->
  MigrationPlan`: `Verdict` (Eclipsed/BackfillInPlace/ReDerive/SkeletonChange), `CostClass`
  (`WriteScope` × `reads_upstream`), `ColumnGroupPlan`/`TechniqueCandidate`. Exhaustive over
  `Technique` (compile error on a new variant). Re-exported from `backbuild/mod.rs`.
- `crates/smelt-state/src/schema_tracking.rs` — `DeployedSchema::definition_sql: String`
  (`#[serde(default)]`, fail-closed empty on legacy snapshots), populated at both save sites in
  `crates/smelt-runtime/src/schema_evolution.rs` from `model_sql`.
- `crates/smelt-runtime/src/migrate.rs` — `ModelMigrationFacts`, `MigrateError`,
  `derive_migration_plan_for_model`: parses recorded vs. current SQL, calls
  `definition_diff` → `derive_backbuild_options` → `derive_migration_plan`. No backend calls.
- `crates/smelt-cli/src/commands/migrate.rs` + `smelt migrate <model> [--project-dir] [--target]`
  CLI verb: loads the project, resolves the model, assembles facts (deployed schema, inferred
  current columns, declared `unique_key`, one `SourceRef` per direct upstream model), prints the
  plan. Executes nothing.
- Spec: `docs/specs/definition_deltas.md` §Known Divergences narrowed (synthesis layer's plan
  half is wired; execution/`--apply` is not); `docs/specs/schema_evolution.md` and
  `docs/specs/run_state.md` document the `definition_sql` field.

## Decisions

- **After-side SQL is raw model text, not compiled SQL.** `execute.rs` always records
  `model.content` (never a type-cast/ref-resolved form) as `definition_sql` — confirmed by
  reading every `ModelPlan` construction site. `smelt migrate` diffs `model.content` against the
  recorded `definition_sql` directly; an earlier draft used `SqlCompiler::compile()`'s
  `_smelt_typed`-wrapped output and every unchanged model spuriously diffed against itself.
- **`SourceRef` per direct upstream *model*, not a full FROM-tree alias walk.** `DependencyGraph::get_upstream`
  gives direct upstream names; a real per-alias walk (`smelt_logical::analysis::walk::QueryTree::from_select`)
  would distinguish self-joins and multi-alias references but doesn't exist yet for this purpose
  and wasn't needed for this phase's tests. Scoped out — see below.
- **No `--database` flag on `MigrateArgs`.** The phase-1 plan text listed it, but this command
  never opens a backend connection (plan-only), so it would be a genuinely dead field. Deferred
  to phase 2 (`--apply` needs a real backend).
- **Verdict mapping:** self-derived add/rename/rewrite → `BackfillInPlace`; every other admitted
  technique, or an atom with zero admissible options (refused only), → `ReDerive`; a skeleton
  atom → `SkeletonChange` unconditionally, regardless of its (always-empty) option set.

## For the next planner

- **Phase 2 (`--apply` + approval store)** needs: a real backend connection for `smelt migrate`,
  plan-hash computation over the plan *data* (already-decided scope, outcome.md Decision log),
  and per-technique statement execution — `BackbuildOption::statements` already exist per
  candidate, unused today (`migrate.rs` prints counts only).
- **`SourceRef` construction is the weakest link** for anything beyond a single-FROM-source
  model: multi-alias/self-join models will get an incomplete `sources` map from `smelt migrate`
  today (falls back to refusal/full-refresh rather than mis-deriving). A FROM-tree alias walk
  (`walk::QueryTree::from_select` → `InputItem::Table`) exists in `smelt-logical` and should be
  wired into `commands/migrate.rs` before B3/B4/D2 (upstream-pull-through/join-enrichment)
  techniques get real-world exercise — flagged, not fixed, here.
- **Physical name for upstream sources**: currently the bare graph node name (works for models;
  raw `sources:`-declared tables get no `not_null_columns`/`unique_key` since they have no
  `FileStore` snapshot and `Config::get_unique_key` only covers models — `SourceInfo.unique_key`
  from `smelt_core::sources` was not wired in this phase).
- Untouched by this phase (later phases per outcome.md): `--apply`, `smelt rebuild` rename,
  conformance-suite definition-edit step kind, the atomicity divergence, the diagnostic rename,
  the docs-site migration guide, `/smelt:validate` closure.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full workspace
  `cargo test`, `example_diagnostics`); required updating `.claude/hardening-baseline.txt`
  (println/expect counts grew — legitimate `smelt-cli` user-facing output and the mirrored
  `.expect("workspace/project not initialized")` idiom) and `.claude/unknown-census.toml` line
  numbers (shifted by an unrelated field addition in `schema_tracking.rs`).
- `cargo test -p smelt-state` — PASS (273+3+5 tests, including the two new
  `deployed_schema_round_trips_definition_sql` / `legacy_snapshot_without_definition_sql_reads_empty`).
- `cargo test -p smelt-logical` (incl. `--test walk_coverage`) — PASS.
- `cargo test -p smelt-cli --test migrate` — PASS (3 tests, real DuckDB fixture project).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — PASS.
- `cargo test -p smelt-lsp --test example_workspaces` — PASS (34 tests).
