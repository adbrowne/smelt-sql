# Phase 6 summary — append-only posture: late append vs violation

## Shipped

- `smelt_logical::maintenance::emit::late_appends` — pure classifier (new
  `CurrentPartitionState`/`LateAppend` shapes) mirroring
  `contract::frozen_horizon::late_arrivals`: a closed partition's row-count
  increase is a late append; a decrease, an unchanged count, or the still-open
  partition are never late appends.
- `emit_append_only_posture_probe`'s SQL predicate narrowed: the fingerprint
  leg now also requires `current_count = recorded_count`, so a closed
  partition's pure count increase no longer trips the violation probe even
  though its fingerprint necessarily changed too.
- `smelt-runtime::source_probes::dispatch_and_record_append_only_postures`
  classifies every held verification's snapshot against the carried baseline
  via `late_appends`, `tracing::warn!`s one line naming the partitions, and
  sets `ProbeRecord.observed` to the late-append partition count. Both
  `execute.rs` dispatch sites (incremental-batch and full-refresh) get this
  for free — same function, no new call site.
- `render_smelt_yml_for` flipped from `probes: {cadence: off}` to
  `cadence: per_run` — the workaround is retired, not scoped down, per the
  full-suite proof below.
- New tests: pure-classifier unit tests (`append_only_posture_classification.rs`,
  5 cases), a live-DuckDB `probe_execution.rs` case proving a pure late
  append no longer fires the SQL predicate, three `smelt-runtime`
  end-to-end cases (late append holds; delete and in-place-update at closed
  partitions still fail), and a new generative case
  `probes::late_append_schedule_holds_with_probes_on` that retries for an
  `AppendLateRow`-bearing schedule and drives it through `execute_project`
  with probes on.
- Spec updated: `model_properties.md` (probe-obligation table row + deleted
  the "does not yet distinguish" Known Divergences bullet),
  `sources.md` (§Semantics 4, `SourceMutationProfileViolated` row),
  `run_state.md` (`observed` example).

## Decisions

- The cadence flip is **global**, not append-only-pool-scoped: all 80
  `maintenance_conformance` tests (the full suite, not just the append-only
  pool) pass with `cadence: per_run` — the plan's escape hatch ("keep the
  flip scoped... if a pool unrelated to late appends fails") was not needed.
- `partition_residue_probes.rs`'s ratchet is unaffected (still 2 tests, no
  numeric ratchet in that file) — this phase closes an append-only-posture
  bullet, not one of that file's partition-grain bullets, matching phase 5's
  precedent for key-grain bullets.
- `observed` is set to `Some(appends.len())` on every `Held` verification
  (including `Some(0)`), not left `None` — the count is always computed now
  that classification runs on every held probe, so there is no reason to
  omit it.

## For the next planner

- Phase 7 (final phase) still needs to sweep `incremental_shapes.md`,
  `model_properties.md`, and `diagnostics.md` for any remaining Known
  Divergence bullets this outcome created that phases 1-6 did not already
  close inline, then run `/smelt:validate` on all four spec anchors.
- The pre-existing full-workspace `cargo test` run intermittently threw 4
  `smelt-lsp::example_workspaces` "Timeout waiting for response to
  initialize" failures once, under heavy concurrent load from a prior
  `maintenance_conformance` run still settling — confirmed a resource-
  contention flake (immediate re-run: 35/35 green standalone, and the full
  `verify-phase.sh` gate green on a clean re-run). Not this phase's bug; no
  action taken beyond re-running the gate.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test append_only_posture_classification --test emit_statements --test probe_execution --test probe_obligation` — 5+61+14+6 passed.
- `cargo test -p smelt-runtime --test source_probes --test statement_parity` — 7+37 passed.
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance` — 80 passed (full suite, cadence-on).
- `cargo test -p smelt-cli --test example_diagnostics` — 122 passed, 1 ignored.
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
