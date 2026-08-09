# Phase 6 summary — runtime lowering for the repair family

## Shipped

- `crates/smelt-runtime/src/maintenance_driver.rs`: `resolve_live_per_group_recompute_cell`
  (scans `NewData`/`UpstreamMutation` cells for `Technique::PerGroupRecompute`, same
  `unaddressed_technique_pin` + `resolve_cell_choice` gating as the sibling resolvers),
  `repair_cell_key` (fail-loud on `RowIdentity::WholeRow`/empty key), pure builders
  `repair_affected_keys_select`/`repair_candidate_select`/`repair_staged_relation`, and
  `execute_per_group_recompute` (emitter → `retry_backend_call` → `execute_statement_group`).
- `crates/smelt-runtime/src/execute.rs`: the keyed run loop's window-forward branch now
  resolves a live repair cell and, when one exists and the target table already exists,
  dispatches `execute_per_group_recompute` **instead of** `execute_cumulative_aggregate` (a
  repair cell is an alternative to `KeyedFold` for the same `NewData` trigger, not a technique
  run alongside it — see the inline doc comment for why this differs from the
  column-scoped-merge/membership-recompute "alongside" dispatch shape). New
  `per_group_recompute` strategy label.
- `crates/smelt-runtime/src/diagnostics.rs`: `build_technique_statements` now takes `cell: &PlanCell`
  (was `trigger: &Trigger`); the `PerGroupRecompute` preview arm builds real illustrative
  statements via the same task-2 helpers.
- Tests: `crates/smelt-runtime/tests/repair_lowering.rs` (new, 6 tests), `statement_parity.rs`
  (+1 executed-vs-emitted leg), `diagnostics.rs` (+1 preview test).
- `docs/specs/incremental_models.md` §Known Divergences: narrowed to name only `diff_patch`
  routing (phase 7) as outstanding.

## Decisions

- Routing site deviates from the plan's literal wording ("after column-scoped-merge and
  membership-recompute, before the region-recompute default"): the keyed branch has no
  region-recompute default, and a repair cell serves the *same* `NewData` trigger the fold
  already covers, so it displaces `execute_cumulative_aggregate` rather than running after it.
  The three techniques structurally cannot contend for one source (repair is derived only for a
  clocked mutable source; `UpstreamMutation` cells only for an unclocked one).
- `repair_affected_keys_select` takes `clamp: Option<&ScanClamp>` + a `Region` (reuses
  `widened_scan_predicate`, previously test-only, now with its first production consumer) rather
  than the plan's bare 3-arg sketch.
- Region endpoints for the repair read are typed `TIMESTAMP '…'` literals, not the bare quoted
  strings every other `Region` site uses — the only place a region endpoint is an arithmetic
  operand under DuckDB's binder.

## For the next planner

- The diagnostics preview names the affected-key source table via the default
  `<schema>.sources_<segs>` mapping; a source with a `name:` override renders a wrong (but
  never-executed) illustrative table. Live runs use `SourceInfo::db_name_for_target` correctly —
  only the preview is affected. Not a success-criteria blocker but worth a follow-up.
- No shipped example workspace reaches the repair family; all six new DuckDB test legs stage
  their own mutable-clocked `raw.orders` source into a temp copy of `examples/timeseries`. If
  `maintenance_conformance`'s generative recipe pool is meant to cover the repair family, it
  needs a real fixture — flagged for phase 8 (conformance recipes).

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy, workspace tests, example_diagnostics)
- `cargo test -p smelt-runtime --test repair_lowering --test statement_parity --test technique_lowering --test diagnostics` — 6/20/27/10 passed, 0 failed
- `cargo test -p smelt-cli --test maintenance_conformance --test explain --test explain_model` — 4/26/53 passed, 0 failed
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed, 0 failed
