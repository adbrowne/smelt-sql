## Drift Report: incremental_shapes

**Spec**: docs/specs/incremental_shapes.md (last_reviewed: 2026-09-04)
**Date**: 2026-09-04
**Scope**: partition-grain residues closed by `docs/outcomes/20260815-partition-grain-residue`
phases 2–7. This is a scoped validation, not a full-spec drift sweep — the key-grain half of
the spec is out of scope (owned by `docs/outcomes/20260815-keyed-grain-residue`, currently
`blocked`).

### Automated checks
- cargo fmt — PASS
- cargo clippy (both feature sets) — PASS
- cargo test (workspace) — PASS
- example_diagnostics — PASS

### Surface drift
- ✅ `timeseries:` / `safety_overrides:` per-`ModelDef` overrides (phase 4) — documented at
  `docs/specs/incremental_shapes.md:161`, implemented in `MODEL_DEF_FIELDS`.
- ✅ Monotone-integer `partition_column` support (phase 5a/5b) — no new Surface syntax (the
  axis is inferred from `resolved_model_schema`), rule 8a already documents `--period` bounds
  reading in the axis's own domain.
- ✅ `smelt explain --json` `source_bounds.{scan_start,scan_end,scan_unresolved}` (phase 6) —
  documented in `docs-site/docs/reference/smelt-explain.md`.
- ✅ Editor-hover clamp readout (phase 6) — was undocumented in
  `docs-site/docs/guide/editor-features.md`; **fixed this phase** (added a paragraph under
  "Hover Information").
- ❌→✅ `MaintenancePartitionColumnChanged` (phase 7) — `docs/specs/diagnostics.md` names
  `incremental_shapes.md` §"The partition grain" as owner, but the code was missing from this
  spec's own partition-grain codes table. **Fixed this phase** (row added).
- ✅ References → Code/Tests/User docs for "The partition grain" were stale (dated to before
  phases 2–7): missing `temporal.rs` subquery descent, `windowing.rs` axis types,
  `resolve_scan_window`, `maintenance/derive.rs`, `schema_tracking.rs`, `model_source_clamps`,
  the two `partition_residue_probes.rs` test files, and the two doc pages. **Fixed this phase.**

### Semantics drift
- ✅ Function-registry-threaded classification (phase 2) — covered by
  `probe_modeldef_per_model_override` and unit tests in `smelt-logical/src/analysis/temporal.rs`
  and `crates/smelt-logical/src/rules/safety.rs`'s `build_model_graph` call site.
- ✅ CTE-only `event_time_column` detection (phase 3) — covered by
  `probe_cte_only_event_time_column`, `rule_diagnostics.rs` unit tests.
- ✅ Per-`ModelDef` overrides (phase 4) — covered by `probe_modeldef_per_model_override`.
- ✅ Integer-axis run path (phase 5a/5b) — covered by `probe_integer_partition_column_run`
  (first-run/backfill/steady-state vs. full-refresh oracle).
- ✅ Per-source clamp observability (phase 6) — covered by
  `probe_explain_json_run_relative_source_bounds`.
- ✅ `partition_column` rename refusal (phase 7) — covered by
  `probe_partition_column_rename_refusal`.
- All six probes above are exercised through the real `smelt` binary (not mocked), matching
  the spec's fail-loud discipline.

### Invariant drift
- ✅ Maintenance-plan purity — `resolve_scan_window` lives in `smelt-logical`, consumed by both
  `inject_source_filters` (runtime) and `explain --json` / LSP hover (observability), per
  phase 6's decision log; no duplicated arithmetic found.
- ✅ Property composition walk — `temporal.rs`'s subquery/CTE descent (phase 2) still runs
  inside the shared bottom-up walk; `walk_coverage` gate green.
- ⚠️ A window function nested inside a `CASE` arm is still invisible to `analyze_expr_temporal`
  — pre-existing, explicitly out of scope for this outcome (advisory-only, no residue bullet).
  Not re-flagged as new drift.

### Timeless-oracle drift
- ✅ No `Phase [A-Z0-9]+` matches in `docs/specs/incremental_shapes.md` body, or in
  `docs-site/docs/guide/editor-features.md`, `docs-site/docs/reference/smelt-explain.md`,
  `docs-site/docs/reference/timeseries.md`.

### Freshness
- last_reviewed: 2026-09-04 (already current; no bump needed)
- most recent code change to referenced paths: 2026-09-04 (this outcome's phases 2–7)
- Verdict: fresh

### Ratchet
- New standing gate: `cargo test -p smelt-cli --test partition_residue_probes --features duckdb
  partition_grain_residues_stay_closed` — parses §"The partition grain" Known Divergences out of
  the spec and asserts its bullet set is exactly the six this outcome does not own. Verified
  red-first against an empty expected set (failed naming all six real bullets), then corrected.

### Findings not fixed (out of this phase's scope)
- The six remaining partition-grain Known Divergences bullets (per-column `data_latency`,
  non-deterministic row-set-membership, schema-evolution-is-a-definition-delta,
  `PartitionGrainForbidsMetrics`, sub-`g_part` suggestion, `NOW()`/`CURRENT_*` pinning) are
  confirmed still open and correctly excluded from this outcome per the phase-1 audit — none
  cites a pre-`docs/outcomes/` tracker this outcome owns.
- Key-grain half of the spec not validated (separate, blocked outcome).
- The `CASE`-nested-window gap in `temporal.rs` (advisory-only) — recorded, not actioned.

### Summary
- Drift items: 2 total (both fixed this phase) — 2 surface (stale §References, missing
  diagnostic-table row), 0 semantics, 0 invariants.
- Recommended next step: none for the partition grain. `/smelt:validate incremental_shapes`
  can be re-run scoped to the key grain once `docs/outcomes/20260815-keyed-grain-residue`
  unblocks.
