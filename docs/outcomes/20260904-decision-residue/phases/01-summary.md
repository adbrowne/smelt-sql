# Phase 1 summary — `PartitionGrainForbidsMetrics`

## Shipped

- `RuleDiagnosticCode::PartitionGrainForbidsMetrics` + `check_partition_grain_forbids_metrics`
  leaf classifier (`crates/smelt-logical/src/rules/rule_diagnostics.rs`): walks every descendant
  `FUNCTION_CALL` in the model's (frontmatter-stripped) body, CST-based, matching a namespaced
  `smelt.metric(...)` call. Wired as the first check in `IncrementalRule::detect`, ahead of
  event-time injectability and batch-safety.
- `DiagnosticCode::PartitionGrainForbidsMetrics` (`crates/smelt-db/src/diagnostics_types.rs`),
  mapped in `rule_diagnostic_code` (`crates/smelt-db/src/lib.rs`).
- LSP code string `"partition-grain-forbids-metrics"` (`crates/smelt-lsp/src/backend.rs`,
  exhaustive match — compiler-forced parity).
- `examples/broken/models/partition_grain_forbids_metrics.sql` fixture.
- Catalogue row in `docs/specs/diagnostics.md`; deleted the "refusal is unimplemented" Known
  Divergence bullet in `docs/specs/incremental_shapes.md` (the normative rule at
  §"Functions inside partition-grain bodies" needed no edit — it already stated the refusal).
- 4 unit tests in `rule_diagnostics.rs`, 2 Salsa-direct tests
  (`crates/smelt-db/tests/partition_grain_forbids_metrics.rs`), 1 CLI example_diagnostics test,
  updated `partition_residue_probes.rs` ratchet (4 → 3 surviving bullets).

## Decisions

- Followed the outcome's decision log: deleted the divergence bullet in this same commit rather
  than deferring to phase 7, since shipping the refusal while the spec still calls it
  unimplemented would be a false spec at this commit.
- Anchored the diagnostic at the model body start (`rowan::TextSize::from(sql_offset)`), matching
  every other rule diagnostic in the same `detect_builtin_rules` loop — no new anchoring
  machinery needed.
- Message includes the metric argument text when it's present in the call, for author
  legibility; falls back to a generic message when the call has no arguments.

## For the next planner

- Phase 2 (sub-`g_part` refusal naming the coarsened window) is next in the table and unblocked.
- The `partition_grain_residues_stay_closed` ratchet's doc comment (line ~420) still says "the
  six this outcome does not own" — that count was already stale before this phase (the table had
  4 entries, not 6) from an earlier outcome's edit. Not fixed here since it's outside this
  phase's task list; worth a one-line fix whenever that file is next touched.
- No other Known Divergence bullets reference `PartitionGrainForbidsMetrics` outside historical
  outcome/plan records (which stay untouched per convention).

## Gates

- `cargo test -p smelt-logical --lib rule_diagnostics` — 27 passed.
- `cargo test -p smelt-db --test partition_grain_forbids_metrics` — 2 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 121 passed, 1 ignored.
- `cargo test -p smelt-cli --test partition_residue_probes` — 1 passed (targeted; full file not
  re-run here but unaffected by this phase's edits beyond the ratchet).
- `cargo test -p smelt-logical --test walk_coverage` — 8 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed.
- `cargo test -p smelt-lsp --test example_workspaces` — 35 passed.
- `cargo fmt --all -- --check` — clean.
- `bash .claude/scripts/clippy-gate.sh` — clean, both feature sets.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
