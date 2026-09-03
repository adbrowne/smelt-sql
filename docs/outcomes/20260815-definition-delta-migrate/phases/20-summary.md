# Phase 20 summary — Mutation-happened discrimination

**Shipped:**
- `docs/specs/incremental_models.md` §"Per-cell admission": new "When a mutation cell
  dispatches" paragraph; deleted the "Plan-consumer gaps" Known Divergences bullet outright
  (it named only this clause).
- `smelt_logical::maintenance::emit::emit_source_mutation_fingerprint` (`crates/smelt-logical/
  src/maintenance/emit.rs`): whole-source (unpartitioned) COUNT+order-independent-fingerprint
  emitter, one per `MaintenanceDialect`, sharing `row_fingerprint_expr`. 4 new tests in
  `crates/smelt-logical/tests/emit_statements.rs`.
- `smelt_state::source_mutations::{SourceMutationBaseline, SourceMutationStore}` +
  `FileStore::{load,save}_source_mutations` (`.smelt/targets/<target>/source_mutations.json`).
  2 unit tests + 1 `FileStore` round-trip test.
- `smelt_runtime::mutation_probe`: pure `decide_mutation_dispatch` (5 unit tests), the
  backend-executing `probe_source_mutation_fingerprint`, and the combining
  `gate_upstream_mutation_dispatch`.
- `crates/smelt-runtime/src/execute.rs`: every live `UpstreamMutation` dispatch site now routes
  through `resolve_upstream_mutation_gate`/`record_upstream_mutation_baseline` — the keyed
  branch's column-scoped-merge site, the keyed branch's membership-recompute site, and the
  non-keyed branch's column-scoped-merge site (gated once per run, before its per-batch loop,
  since the source fingerprint doesn't change across a run's own batches). Recording happens
  only after the licensed technique's write actually succeeded. Fixed the stale ~L2058 comment
  (phase-19 summary flagged it) claiming `UpstreamMutation` is derived only for an unclocked
  source — it now names the real narrowing (`derive_mutation`'s repair-cell exclusion).
- New tests: `statement_parity.rs::source_mutation_fingerprint_comes_from_the_emitter`,
  `technique_lowering.rs::column_scoped_merge_skipped_when_dimension_unmutated`.

**Decisions:**
- Digest columns for the fingerprint are the source's declared YAML `columns:` (same convention
  `append_only_posture_probes` already uses) — a source missing declared columns gets no gate at
  all (treated as always-dispatch, matching the "no baseline" fail-open posture) rather than a
  refusal, since there's nothing to fingerprint.
- The non-keyed branch's `column_merge_dispatch` decision is gated ONCE before the per-batch
  loop (not per-batch) since the source's whole-source fingerprint is a run-level fact; a `NoOp`
  verdict overrides the dispatch to `None` for the whole run, so every batch falls through to
  the ordinary DELETE+INSERT path exactly as if no live cell had resolved. The keyed branch's
  two sites dispatch immediately (single window, no per-batch loop) so the gate wraps them
  directly.
- Closing this divergence changed observable behavior in 3 existing tests that had encoded the
  OLD "known divergence — unconditional per-run dispatch" as their expected assertion:
  `technique_lowering.rs`'s `keyed_run_loop_dispatches_membership_recompute_through_execute_project`
  and `diff_patch_pin_over_region_delete_insert_default_writes_only_the_difference` (both: an
  unchanged-since-last-run third run now reports `cumulative_aggregate`, not
  `delete_insert_suppressed`/`diff_patch`), and `maintenance_conformance/gate.rs`'s
  `value_enriched_recipe_executes_column_scoped_merge` (redelivery window now `deleteinsert`,
  not `column_scoped_merge`) and `keyed_enriched_pool_upholds_equivalence_under_dim_mutation`
  (fuzzed post-creation windows that never touch the dimension now assert
  `cumulative_aggregate` from window 2 onward, and the hand-built zero-change redelivery window
  now asserts `cumulative_aggregate` too). All were updated with doc comments explaining the new
  behavior; none needed a different execution or oracle — the equivalence checks all still pass,
  since a no-op cell writes nothing, same end state as a dispatch that finds nothing to change.

**For the next planner:**
- Phase 21 (Graph-layer gaps) is next in the table.
- Not touched: the third-run "known divergence" pattern may exist in other maintenance
  conformance tests beyond the two files fixed here — only failures the standing gates actually
  surfaced were fixed; a broader sweep for stale "always dispatches" assertions elsewhere
  (e.g. `smelt-cli/tests/maintenance_pins.rs`, `bakeoff*.rs`) was not performed and could turn up
  more.
- Not touched: per-group recompute (repair family) and key-addressed model-edge cells are NOT
  gated by this mechanism — they are a different trigger space (`Trigger::NewData` repair cells,
  keyed on the model's own driving trigger or an upstream model's bare name), not
  `Trigger::UpstreamMutation`, so `derive_mutation`'s narrowing already prevents the same source
  from carrying both kinds of cell. If a future phase ever makes repair cells re-checkable this
  way, it needs its own baseline store — this one is scoped to `UpstreamMutation` only.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test maintenance_plan_admission` — 11 passed.
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test execute_parity`
  — 4 + 25 + 32 passed.
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 74 passed.
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
