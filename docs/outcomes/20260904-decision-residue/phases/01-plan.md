# Phase 1 — `PartitionGrainForbidsMetrics`

**Outcome:** `docs/outcomes/20260904-decision-residue`
**Advances:** success criterion 1 (and its slice of 8/9).

## Objective

A partition-grain model whose body calls `smelt.metric(...)` must refuse ahead of execution with
`PartitionGrainForbidsMetrics` from `file_diagnostics()`, reaching CLI and LSP identically through
the existing rule → diagnostics seam. Key-grain, full-refresh and view models with the same call
are untouched. The spec's "refusal is unimplemented" Known Divergence bullet goes with it.

## Spec delta

1. `docs/specs/diagnostics.md` — add a catalogue row (Error severity) for
   `PartitionGrainForbidsMetrics` near the other partition-grain rows: a `grain: partition`
   model's body consumes `smelt.metric()`; the composition of metric expansion with time-filter
   injection is deliberately unspecified, so the combination refuses. Required by the
   `every_diagnostic_code_is_catalogued` gate.
2. `docs/specs/incremental_shapes.md` §Known Divergences → "The partition grain" — delete the
   bullet "**The `PartitionGrainForbidsMetrics` refusal is unimplemented**". §"Functions inside
   partition-grain bodies" already states the rule normatively and needs no edit.
   (Reshape note: this bullet's deletion moved out of phase 7 into this phase so no commit ships
   a spec that calls an implemented refusal unimplemented.)

## Tests (red → green)

- `smelt-logical` `rule_diagnostics::tests::partition_grain_metric_call_refuses` — a
  `grain: partition` `RuleContext` whose SQL calls `smelt.metric('revenue')` yields exactly one
  `RuleDiagnosticCode::PartitionGrainForbidsMetrics` at `RuleSeverity::Error`.
- `…::partition_grain_metric_call_in_cte_refuses` — the call nested in a CTE / derived table is
  found too (descendant walk, not an outer-select-only scan).
- `…::partition_grain_without_metric_is_clean` — a partition-grain model calling `SUM`/a
  `smelt.<path>` ref produces no `PartitionGrainForbidsMetrics`.
- `…::keyed_model_metric_call_is_unaffected` — the same body under `materialization` =
  keyed/`full_refresh` produces no `PartitionGrainForbidsMetrics`.
- `smelt-db` `tests.rs` `partition_grain_metric_call_file_diagnostic` — Salsa-direct
  `file_diagnostics()` over a partition-grain model with `smelt.metric()` contains exactly one
  `DiagnosticCode::PartitionGrainForbidsMetrics`, Error severity, anchored in-range.
- `smelt-cli` `example_diagnostics::broken_workspace_partition_grain_forbids_metrics` —
  modelled on `broken_workspace_maintenance_scan_unbounded`: exactly one
  `PartitionGrainForbidsMetrics` from `models/partition_grain_forbids_metrics.sql` and none from
  any other file in `examples/broken/`.
- `smelt-cli` `partition_residue_probes::partition_grain_residues_stay_closed` — update
  `expected_leads` to the three surviving bullets (red until the spec bullet is deleted).

## Tasks

1. Add `PartitionGrainForbidsMetrics` to `RuleDiagnosticCode`
   (`crates/smelt-logical/src/rules/rule_diagnostics.rs`) with a doc comment stating the refusal
   and its spec anchor.
2. Write `check_partition_grain_forbids_metrics(ctx) -> Option<RuleDiagnostic>`: parse
   `Frontmatter::strip(ctx.sql)`, walk **all** descendant `FUNCTION_CALL` nodes, and match the
   namespaced call whose token run is `IDENT("smelt") DOT IDENT("metric")` (case-insensitive).
   CST-based — no `.contains("` text scan, so the `walk_coverage` gate stays clean; the doc
   comment classifies it as a leaf classifier over the model's own body per the property
   composition walk rule. Message names the model and the metric argument text if present, and
   points at §"Functions inside partition-grain bodies".
3. Call it as the **first** check in `IncrementalRule::detect` (before
   `check_event_time_injectable`) and return immediately — an unspecified composition should not
   be reported behind a batch-safety warning.
4. Add `DiagnosticCode::PartitionGrainForbidsMetrics` to
   `crates/smelt-db/src/diagnostics_types.rs` with the message/severity doc comment in house
   style; map it in `rule_diagnostic_code` (`crates/smelt-db/src/lib.rs:1254`).
5. Add the LSP code string `"partition-grain-forbids-metrics"` in
   `diagnostic_code_str` (`crates/smelt-lsp/src/backend.rs`) — the exhaustive match makes this
   compiler-forced parity.
6. Add fixture `examples/broken/models/partition_grain_forbids_metrics.sql`: `refresh:
   incremental`, `grain: partition`, a `timeseries:` block over an existing broken-workspace
   source, a body selecting `smelt.metric('...')`, and a comment citing the spec section.
7. Make the spec edits from §Spec delta; update the `expected_leads` ratchet in
   `crates/smelt-cli/tests/partition_residue_probes.rs` (drop the now-closed bullet and adjust
   its count/message).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-cli --test partition_residue_probes`
- `cargo test -p smelt-cli --test example_diagnostics`
- `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(incremental): refuse smelt.metric() in partition-grain bodies with PartitionGrainForbidsMetrics`
