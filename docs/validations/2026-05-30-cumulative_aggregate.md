# Drift report — `cumulative_aggregate`

**Date**: 2026-05-30 · **Phase**: B3 · **Spec**: `docs/specs/cumulative_aggregate.md`
**Method**: spec-vs-code read + adversarial fixtures (`examples/cumulative_classifier_gate`) driven through `smelt run`/`build` + DuckDB, plus existing `cumulative_equivalence` / planner / metadata gates.

## Verdict

The feature is **substantially implemented and the cross-partition equivalence
contract holds** (forward / reverse / shuffled orderings all match a full
refresh — `crates/smelt-cli/tests/cumulative_equivalence.rs`, green). The
classifier (`smelt-planner::classify_cumulative`) implements every Surface
§"Diagnostic codes" rule and every Semantics §"Classifier checks" rule, with
unit coverage. One **correctness bug fixed in-loop** (BUG-010) and one
**needs-review** drift (BUG-011) found.

## Surface coverage

| Surface element | Status | Notes |
|---|---|---|
| `materialization: cumulative_aggregate` frontmatter | ✅ | `Materialization::CumulativeAggregate`; parses, builds end-to-end |
| `smelt.yml` `materialization:` override | ✅ | shares the metadata path |
| forbid `timeseries:` (`CumulativeForbidsTimeseries`) | ✅ | `metadata.rs:333`, tested |
| forbid `incremental:` (`CumulativeForbidsIncremental`) | ✅ | `metadata.rs:323`, tested |
| `--event-time-start/-end` window | ✅ | windowed run steps partitions + `MERGE INTO` |
| Aggregator allowlist (COUNT/SUM/MIN/MAX/BOOL_*/BIT_*) | ✅ | `combiner_for`, unit-tested incl. case-insensitivity |
| Diagnostic codes (10) | ⚠️ | All produced by the classifier, but only the 2 metadata-level forbid codes surface at workspace-load / LSP; the 7 classifier-check codes fire **only at `smelt run`/`build`**, never in the LSP/diagnostics (`smelt-db`) layer — **BUG-011** |

## Semantics coverage

| Rule | Status | Notes |
|---|---|---|
| Execution model (per-partition step + merge_into) | ✅ | `cumulative.rs::execute_cumulative_aggregate` |
| Cross-partition equivalence (any ordering) | ✅ | `cumulative_equivalence.rs` (fwd/rev/shuffled), green |
| Classifier checks (GROUP BY, allowlist, partition-col, window, nondeterministic) | ✅ | `classify_cumulative`, unit-tested |
| Driving-source cardinality (0/1/≥2) | ✅ | unit-tested |
| Output shape (no partition_column) | ✅ | output is one row per key |
| **Constraint #10 — no silent downgrade** | ❌→✅ | **BUG-010**: the no-window full-refresh path bypassed the classifier entirely, silently materialising forbidden SQL (e.g. `STRING_AGG`) as a plain table (exit 0, no diagnostic). Fixed in-loop — classifier now runs on the no-window path in both run-pipeline entry points. |
| Reprocessing refusal (v1) | ⚠️ (matches spec) | runtime comment notes the watermark check is a placeholder; the spec's §"Reprocessing semantics" detection is acknowledged-conservative — consistent with §Known Divergences, not drift |

## Findings

- **BUG-010** (fixed) — no-window path bypassed the cumulative classifier → silent
  full-refresh of forbidden cumulative SQL. Violated Constraint #10. Fixed in
  both `smelt-runtime::execute_project` and `smelt-cli::commands::run` via a new
  shared `smelt_runtime::classify_cumulative_sql` helper; red-green regression in
  `crates/smelt-cli/tests/cumulative_classifier_gate.rs`.
- **BUG-011** (needs-review) — the 7 classifier-check diagnostic codes are not
  surfaced in the LSP/`smelt-db` diagnostics layer; an author editing a malformed
  cumulative model sees no diagnostic until `smelt run`/`build`. Spec frames them
  as diagnostic codes "rejected at planning time". Same asymmetric class as
  BUG-006. See ledger for options.

## Note (no bug, out of phase scope)

`ModelDefInvalidMaterialization` (`smelt-db/src/diagnostics_types.rs:574`) — the
**meta-language** `ModelDef` literal validator restricts `materialization` to
`{view, table, incremental}`, omitting `cumulative_aggregate` (and
`materialized_view`/`ephemeral`/`test`). A generator emitting a cumulative model
via a `ModelDef` literal would be wrongly rejected. This is generator/meta-language
surface (Wave D3/D6), not `cumulative_aggregate`'s frontmatter Surface, and is
unverified here — flagged for the generator phases.
