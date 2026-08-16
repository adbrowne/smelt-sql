# Phase 4 summary — dispatch composition in the run loop

**Shipped:**
- `resolve_live_key_addressed_model_edge_cells` (plural) in `maintenance_driver.rs`, collecting
  every `Technique::PerGroupRecompute` cell with a `key_scope`, one per covered edge. The
  singular `resolve_live_key_addressed_model_edge_cell` is now a delegating wrapper (first cell)
  so pre-phase-4 callers/tests are unchanged.
- `execute.rs`: `key_addressed_edge_cells: Vec<_>` resolved once above `plan_is_keyed`. The keyed
  branch (`~L2047`) dispatches every resolved cell in sequence (each targets the same table, so
  the LAST result's `row_count` is kept, never a sum). The non-keyed site (`~L2703`) is now a
  **coverage gate**: licensed when every inbound ref (declared source or model edge) resolved a
  key-addressed cell; dispatches all of them and sums into one manifest entry / one
  `model_completed`.
- `RunReporter::dispatch_widened` (default no-op) — fires when cells resolved but the coverage
  gate refused, naming the uncovered input(s). `CliReporter` prints it as a warning line.
  `EventSink`/`ReporterEvent::DispatchWidened` wired so the wavefront scheduler's buffer-then-
  replay path actually forwards it (the bug this phase's own e2e test caught — `EventSink`
  silently no-op'd on any `RunReporter` method it doesn't explicitly buffer/replay).
- `docs/specs/incremental_models.md` Known Divergences bullet narrowed per the plan's §Spec delta.
- 6 new tests in `crates/smelt-runtime/tests/key_addressed_model_edge_lowering.rs` (14 total, all
  passing); 2 pre-existing e2e tests re-asserted unchanged (regression pin).

**Decisions:**
- Multi-edge admission (both the unit-test fixture and the e2e fixture) needed a COALESCE/
  composite-grain shape, not a plain equi-join — `derive_affected_keys`'s provenance walk traces
  literal SELECT-list column lineage, not join-predicate equality, so `a.user_id = b.user_id`
  alone never makes the SAME output grain column depend on both sides. Documented inline at both
  fixtures.
- The keyed-branch dispatch loop keeps the LAST cell's `row_count` rather than summing — every
  key-addressed cell targets the SAME `db_table_name`, so a sum would double-count.

**For the next planner:**
- Found and fixed a real bug via the new e2e test, not just added coverage: `EventSink` (the
  wavefront scheduler's per-model event buffer, `execute.rs` `~L163`) silently swallows any
  `RunReporter` trait method it doesn't explicitly buffer and replay — new reporter methods added
  in future phases (e.g. row 8's `smelt explain` surface work) MUST add a matching
  `ReporterEvent` variant + buffer arm + replay arm, or they silently no-op under the real
  concurrent run loop despite working in a bare `NoOpReporter`/single-model test. Worth a doc
  note in `crates/smelt-runtime/CLAUDE.md`'s Gotchas if another phase touches `RunReporter`.
- Fan-in merge across MULTIPLE edges landing on the SAME grain key (not just several edges each
  owning disjoint parts of a composite key) is now exercised by `two_keyed_upstreams_dispatch_
  both_cells`, closing the "implemented but untested" gap phase 3's summary flagged.
- Row 5 (propagated key restrictions reaching the cell) is next; this phase's coverage gate does
  not change row 5's scope — the sidecar-discovered key set is still the only source of the
  affected-key restriction dispatched here.

**Gates:**
- `cargo test -p smelt-runtime --test key_addressed_model_edge_lowering` — 14 passed.
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity --test
  typed_edge_graph` — 4 + 23 + 5 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 76 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo
  test`, example_diagnostics). `smelt-cli println` hardening baseline bumped 163→164 for the one
  new legitimate `dispatch_widened` warning line in `CliReporter` (`hardening-budget.sh
  --update`).
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md` — no matches.
