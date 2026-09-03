# Phase 27b summary — the region DELETE+INSERT family gains its conditional variant

**Shipped:**
- `choice::RegionWrite` (`crates/smelt-logical/src/maintenance/choice.rs`) — `Unconditional { why }` /
  `Suppressed { key, compared_columns }` — plus `resolve_region_write_variant`, composed entirely
  from the existing `resolve_write_suppression` + `resolve_write_variant` (no new proof logic).
  4 unit tests in `region_write_variant_tests`.
- `DeltaRestrictionFacts` (`crates/smelt-runtime/src/maintenance_driver.rs`) gained a `region_write`
  field, resolved in `resolve_live_delta_restriction_facts` from the SAME `Trigger::NewData` cell
  already read there — group columns are the model's own output projection minus the proven
  row-identity's key columns (a model-edge creation cell's `PlanCell::group` is the literal
  whole-row `"{*}"` placeholder, not a named column group). The function's return type changed to
  `Result<Option<DeltaRestrictionFacts>, ChoiceRefusal>` so a `technique: suppress` pin over a
  refused P2/P3 proof surfaces as a real run error.
- `build_delete_insert_group_dispatched` gained a `region_write: Option<&RegionWrite>` parameter
  and a third dispatch arm: `RegionWrite::Suppressed` calls `emit_diff_patch` with
  `region.predicate(Some(table), partition_col)` as the slice predicate and `DeleteLeg::Complete`.
  Delta restriction still wins when both are admitted (unchanged match order). Both the live
  executor (`execute_delete_insert_with_delta_restriction`) and the dry-run reporting loop in
  `execute.rs` thread `region_write` through to this one dispatch point.
- Tests: 4 `choice.rs` unit tests; 3 pure-function tests in `region_choice_ladder.rs`
  (conditional emission, non-regression widened-scan, delta-restriction-wins-over-suppression);
  2 live DuckDB tests in the new `region_conditional_write.rs` (unchanged-data no-op,
  changed/departed/new-key coverage); 1 executed-vs-emitted byte-parity test in
  `statement_parity.rs`; 1 dry-run test in `dry_run_statements.rs` proving the reported and live
  forms cannot diverge (both route through `build_delete_insert_group_dispatched`).
- Spec: `model_transforms.md` gained a paragraph naming the region family's own conditional
  realisation and narrowed the matching Known Divergences bullet; `incremental_models.md`'s
  "Conditional-maintenance gaps" bullet narrowed the same way.
  `docs-site/docs/reference/cli.md`'s `--dry-run` section gained one sentence.

**Decisions:**
- Reused `emit_diff_patch` (not `emit_staged_candidate_conditional`, not a new emitter) per the
  plan's design call — a region recompute's candidate covers its own slice by construction, so
  `DeleteLeg::Complete` is always sound here, unlike the keyed staged-candidate's unbounded
  `DELETE`.
- The staged temp-relation name sanitises `table`'s embedded schema `.` (`table.replace('.', "_")`)
  rather than reusing `diff_patch_staged_relation` verbatim — that helper assumes a bare table
  name, and `build_delete_insert_group_dispatched`'s own `table` parameter is always
  schema-qualified (`emit_delete_insert`'s convention). Caught by a real DuckDB
  `Schema with name __smelt_diff_patch_main does not exist` failure before it shipped.
- `region_write` is derived and consulted ONLY inside `DeltaRestrictionFacts`/
  `resolve_live_delta_restriction_facts` — i.e. only for a model-edge-sourced creation cell
  (`model_edges` non-empty). An external-source-only region cell's `Trigger::NewData` never
  reaches this resolver at all (the plain `backend.execute_model_incremental` path in `execute.rs`
  bypasses `build_delete_insert_group_dispatched` entirely for that case) — this is the literal
  scope the plan's task 3 named ("derived from the SAME `Trigger::NewData` cell it already
  reads"), not an oversight. Recorded honestly in both specs' Known Divergences rather than
  silently narrowing the claim.

**For the next planner:**
- The remaining gap is real and non-trivial: widening region write suppression to
  external-source-only region cells requires wiring through the plain
  `backend.execute_model_incremental`/`MaterializationStrategy::Incremental` path in `execute.rs`
  (the `else` branch after `restricted_facts_this_batch`), which today calls `emit_delete_insert`
  only for the dry-run *report* and executes via a completely different `Backend` trait method —
  not `build_delete_insert_group_dispatched` at all. That is a materially bigger change (a new
  `Backend` capability or a second live dispatch path) and was correctly left out of this phase's
  scope; a future phase should scope it explicitly rather than assume it falls out for free.
- `smelt explain --show-sql` is NOT wired to this new `RegionWrite` dimension — it still prints
  the unconditional widened scan for a region `DeleteInsert` cell (unlike `ColumnScopedMerge`/
  `KeyedFold`, which `resolve_cell_write_suppression` already covers there). Confirmed by grep:
  no `DeltaRestrictionFacts`/`build_delete_insert_group_dispatched` reference in
  `smelt-cli/src/explain.rs`. Row 27c/27d/27e/27f are unaffected by this; a follow-up should
  either extend `explain.rs` or record this as its own divergence if intentionally deferred.
- Row 27c (keyless whole-row `EXCEPT ALL` staged-candidate) is a natural next step now that the
  region family's keyed conditional path is proven end-to-end.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, full workspace
  `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --lib maintenance` — 179 passed.
- `cargo test -p smelt-runtime --test region_choice_ladder --test dry_run_statements --test statement_parity --test delta_restricted_recompute --test technique_lowering --test region_conditional_write` — 73 passed.
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 74 passed.
- `cargo test --workspace` — all green (run separately, full pass).
