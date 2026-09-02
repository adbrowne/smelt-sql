# Phase 17 summary

**Shipped:**
- Fixed `find_keyword_not_in_parens` (`crates/smelt-logical/src/analysis/mod.rs`) to treat `_`
  as an identifier char and reject a match preceded by `.`/`"`/backtick — `order_id` (and
  `having_flag`, `union_all`, `limit_count`, `except_code`, `intersect_key`, `fetch_size`) no
  longer truncate a `GROUP BY` clause at the collision point.
- Frontmatter-time `grain: key` identity check: `crates/smelt-db/src/queries/maintenance.rs`'s
  `ConfigGrain::Key` arm now refuses (new `Refusal::IdentityNotDerivable` →
  `DiagnosticCode::GrainAssertionMismatch`, `crates/smelt-logical/src/maintenance/mod.rs`) a
  model with no declared `unique_key:` and an empty `GROUP BY`-derived key — reaches
  `file_diagnostics()` (CLI + LSP parity) and `smelt explain` without a run.
- `resolve_incremental_strategy` (`crates/smelt-runtime/src/maintenance_driver.rs`) is now
  edge-aware: takes `model_edges: &[ModelEdge]`, derives via
  `derive_model_maintenance_plan_with_edges` when non-empty, and reads the driving edge's own
  cell via `plan.cell_for` instead of the first `NewData` cell in plan order. Returns
  `anyhow::Result` now — a driving edge refused `ReachNotDerivable` with no other creation cell
  is a fail-loud `bail!` naming the edge, not a silent `backend_default`.
- `execute.rs`: hoisted `model_edges_for` above `resolved_strategy` (computed once, shared with
  the existing T3 delta-restriction call site).
- New test file `crates/smelt-runtime/tests/model_edge_creation_cell.rs` (3 tests).

**Decisions:**
- `IncrementalStrategy` has exactly one variant (`DeleteInsert`) today, so "the edge cell and
  the source-driven cell admit visibly different `IncrementalStrategy` values" is not
  observable through the return type — the differentiation the plan asked for is real at the
  `MaintenancePlan`/cell level (verified by inspecting which cell `cell_for` resolves) but
  collapses to the same enum value either way. Adjusted the first test's scope to what's
  actually checkable: a model with only a model-edge input (no plain `sources:`) resolves
  successfully via the edge's own cell rather than falling back to `backend_default` for lack
  of any cell at all (the pre-phase behaviour). Tests 2/3 (refusal + narrow fallback) are
  unaffected by this and exercise the real new branch (`bail!` vs. fall-through).
- Reused the existing `GrainAssertionMismatch` diagnostic code (no new code) by adding a new
  `Refusal::IdentityNotDerivable` variant mapped to it — `LocalityNotEstablished` already owns
  `KeyedForbidsTimeseries`, so a distinct refusal variant was needed to land on the right code.
- `crates/smelt-db/tests/maintenance_diagnostics.rs::grain_mismatch_is_error_never_silent` and
  `crates/smelt-cli/tests/explain_maintenance.rs::degenerate_plan_visibly_reported` both used
  fixtures with `grain: key` + no `unique_key:` + no `GROUP BY` — genuinely underivable
  identity, now caught by the new frontmatter check before the older
  `MaintenanceNoAdmissibleTechnique`/degenerate-collapse paths ever run. Updated the first to
  assert `GrainAssertionMismatch`; gave the second's fixture a `GROUP BY o.order_id` (with
  `MIN(...)` folds) so it keeps a derivable identity and still reproduces the ambiguous-join
  column-group collapse it was actually testing.

**For the next planner:**
- Phase 18 (plan-consumer + graph-layer sweep) is next; nothing else surfaced that serves this
  outcome's success criteria beyond what's already listed there.
- Not investigated: whether other `IncrementalStrategy`-returning call sites would benefit from
  the same edge-aware treatment — today there's only the one call site in `execute.rs`.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, example_diagnostics).
- `cargo test -p smelt-logical --lib analysis` — 304 passed.
- `cargo test -p smelt-db --test maintenance_model_upstream` — 4 passed.
- `cargo test -p smelt-runtime --test model_edge_creation_cell --test statement_parity --test key_addressed_model_edge_lowering` — 9+3+24 passed.
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance --test example_diagnostics` — 74+119 passed (1 ignored), no real fixture newly refused.
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
