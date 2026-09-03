# Drift Report: incremental_shapes (closure sweep)

**Spec**: docs/specs/incremental_shapes.md (last_reviewed: 2026-09-04)
**Date**: 2026-09-04

> This report is a full-spec sweep and supersedes the scope of the earlier
> `docs/validations/2026-09-04-incremental_shapes.md` (partition-grain only, commit 7f4358cf),
> which is left in place for history. This report was produced as steps 3-6 of `/smelt:validate`
> against `docs/outcomes/20260815-incremental-spec-closure-confirm/baseline-inventory.md`, whose
> "## incremental_shapes.md" table (IDs IS-01..IS-32) is treated as already-verified ground truth
> for every Known Divergences / Open Questions bullet — those are cited by ID below rather than
> re-litigated.

### Automated checks

Already run once for this outcome (not re-run here per task instructions):
- cargo fmt --all -- --check — PASS
- cargo clippy --all-targets — PASS
- cargo test — PASS
- example_diagnostics — PASS

### Surface drift

- ✅ `grain: partition` declaration, `timeseries:` requirement, `safety_overrides`, `columns.<c>.contract: plausible` — all present in `crates/smelt-core/src/config.rs`, `crates/smelt-core/src/metadata.rs` (`allow_window_functions` etc. at metadata.rs:881-882, config.rs:676).
- ✅ `grain: key` declaration, `unique_key`/`GROUP BY` restatement, `functional_dependencies:` — present (`crates/smelt-db/src/lib.rs:2146,2523,2831`, `crates/smelt-db/src/queries/maintenance.rs:208-209`).
- ✅ Column-family catalogue's aggregators/combiners — reflected in `crates/smelt-logical/src/rules/cumulative.rs` classifier and covered by `crates/smelt-logical/tests/keyed_families.rs`.
- ✅ All 21 shape-local diagnostic codes in the two §Diagnostics tables (`TimeseriesRequiredForPartitionGrain` … `KeyedRecurrenceBoundViolated`) resolve to real `DiagnosticCode` variants and call sites in `crates/`.
- ⚠️ `PartitionGrainForbidsMetrics` has exactly one hit in `crates/`, inside a test asserting the refusal is *unimplemented* (`crates/smelt-cli/tests/partition_residue_probes.rs:452`) — not a missing-code drift, this is the already-tracked gap — flagged-open: IS-08.
- ⚠️ No `smelt.define` template file ships `smelt.latest`/`smelt.once`/`smelt.current` (searched for `.smelt` files under `define`, and for the call names — no hits) — flagged-open: IS-25.
- ❌ **References drift (fixed this phase)** — `docs/specs/incremental_shapes.md` §References → "The partition grain" → Code cited `crates/smelt-logical/src/windowing.rs` as the home of `PartitionAxis`, `PartitionPoint`, and `resolve_scan_window`. That file does not exist. Actual locations: `PartitionAxis` is `crates/smelt-logical/src/analysis/partition_axis.rs`; `resolve_scan_window` is `crates/smelt-logical/src/analysis/source_bounds.rs`; `PartitionPoint` is `crates/smelt-runtime/src/windowing.rs` (which does exist, and already had its own correct References line for `IncrementalBatch`). — fixed this phase (docs/specs/incremental_shapes.md, lines ~1333-1334).
- ✅ docs-site pages referenced in §References all exist: `guide/incremental-models.md`, `guide/materializations.md`, `reference/smelt-explain.md`, `reference/timeseries.md`, `reference/cumulative-aggregate.md`, `examples/web-analytics/deduplication.md`.

### Semantics drift

- ✅ `g_run >= g_part` granularity ordering — exercised in `crates/smelt-logical/src/rules/incremental.rs` (coarser/coarser-but-buildable verdict logic) and covered by `crates/smelt-cli/tests/incremental_*.rs` batch/backfill suite.
- ✅ `EventTimeColumnNotVisibleAtOuterSelect` — covered by `crates/smelt-logical/tests/partition_residue_probes.rs`, `crates/smelt-cli/tests/example_diagnostics.rs`, `crates/smelt-cli/tests/e2e/source_pushdown_e2e.rs`.
- ✅ `KeyedOnceWriteUnproven` (the four once-write spellings and their FD proof) — covered by `crates/smelt-logical/tests/keyed_families.rs`.
- ✅ Derived execution postures (re-run tolerance / order-independence / reprocessing refusal) — covered by `crates/smelt-logical/tests/execution_postures.rs`; `smelt explain` rendering confirmed in `crates/smelt-cli/src/explain.rs:770-808` per baseline-inventory IS-22.
- ✅ Transactional merge ledger / frontier — covered by `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs`; DuckDB-only transactional fold is the already-tracked gap — flagged-open: IS-18.
- ✅ `arb_once_write_null_schedule` generative NULL-payload coverage — present in `crates/smelt-maintenance-testkit/src/recipe.rs`, consumed by the `maintenance_conformance` gate, matching IS-23's closed disposition.

### Invariant drift

- ✅ Partition-grain constraint 13 (`MaintenancePartitionColumnChanged`, deployed-schema snapshot address) — `crates/smelt-state/src/schema_tracking.rs` (`DeployedSchema::partition_column`) and `crates/smelt-logical/src/maintenance/derive.rs` (`partition_column_changed`, `Refusal::PartitionColumnChanged`) both exist as cited.
- ✅ Key-grain constraint 1 (`safety_overrides:` hard error once key-addressed) — `KeyedForbidsSafetyOverrides` has 21 hits across code/tests; matches IS-17's closed disposition.
- ⚠️ Key-grain constraint 9 (transactional merge ledger written atomically) is verifiably upheld on DuckDB only by inspection of `Backend::execute_write_with_bookkeeping` overrides — the non-DuckDB gap is IS-18 (flagged-open), not re-flagged as a fresh invariant violation.
- ✅ No phase-vocabulary leakage found anywhere in the invariant/constraint prose (see Timeless-oracle drift below).

### Timeless-oracle drift

- ✅ `rg -n 'Phase [A-Z0-9]' docs/specs/incremental_shapes.md` returns **zero matches** — no phase-vocabulary leakage in spec body, Known Divergences, Design, Constraints, or References sections.

### Freshness

- last_reviewed: 2026-09-04
- most recent code change among cited References → Code paths (`incremental.rs`, `cumulative.rs`, `maintenance/derive.rs`, `maintenance_driver.rs`, `cumulative.rs` (runtime), `explain.rs`, `schema_tracking.rs`): 2026-09-04T06:40:57+10:00
- Verdict: **fresh** (last_reviewed same day as most recent cited-code commit).

### Summary

- Drift items: 1 total, all doc/wording drift — 1 surface (stale References file path), 0 semantics, 0 invariants, 0 timeless-oracle, 0 freshness.
- Disposition: the one genuine drift item (§References → "The partition grain" → Code citing a nonexistent `crates/smelt-logical/src/windowing.rs`) was fixed inline in this phase.
- Everything else checked against the spec's Surface/Semantics/Constraints sections is either verifiably upheld with a named test/code site, or already honestly flagged in `docs/outcomes/20260815-incremental-spec-closure-confirm/baseline-inventory.md` (cited by ID: IS-08, IS-17, IS-18, IS-22, IS-23, IS-25) — none of those are re-litigated as new drift.
- No item required a phase row (behaviour drift) or a product decision (blocked).
- Recommended next step: none — spec is fresh and in sync with the implementation, modulo its own honestly-tracked Known Divergences.
