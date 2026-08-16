# Outcome: Close the partition grain's residues (v2, decision-grown)

**Created:** 2026-08-16
**Status:** queued
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 5);
carries forward `docs/outcomes/20260815-partition-grain-residue/` (superseded) and adds the scope
the decision track graduated (`docs/research/20260816-open-questions-triage.md` items 6, 7, 19).
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/incremental_models.md`,
`docs/specs/model_transforms.md`, `docs/specs/model_properties.md`

## The outcome

The partition grain's Known Divergences close against the post-decision-track spec. The headline
is the determinism scope: compile-time pinning of `NOW()`/`CURRENT_*` is removed — volatile
clocks execute as-is, the conformance oracle's comparison exempts the columns they feed via the
per-column determinism verdict, `smelt explain`'s guarantee ledger prints the exemption, and
recompute-equality techniques (change suppression, diff-then-patch) gate on the same verdict.
Alongside it, the carried stale-plan-tracked residues land: classification reads through
`smelt.define` bodies, CTE-hidden `event_time_column` is caught ahead of execution,
generator-emitted models get per-model overrides, monotone-integer partition columns run
end-to-end, clamp observability finishes, a `partition_column` rename refuses with a named
diagnostic, sub-partition run windows are rejected with the coarsened suggestion spelled out,
and `smelt.metric()` in a partition-grain body refuses with `PartitionGrainForbidsMetrics`.

## Success criteria (checkable)

1. **The determinism scope is live.** Compile-time pinning is removed from the transformer
   (`model_transforms.md` §"Volatile clocks run as-is"); the conformance oracle's comparison
   exempts columns the per-column determinism verdict marks volatile; `smelt explain`'s
   per-column guarantee ledger prints the determinism exemption; suppression/diff techniques
   refuse or exclude volatile columns via the same verdict (no phantom-drift repair churn).
   Closes `incremental_models.md` "The determinism scope is unimplemented" and
   `incremental_shapes.md` "`NOW()`/`CURRENT_*` are still compile-time-pinned" and
   `model_transforms.md` "The implementation still compile-time-pins volatile clocks."
2. A sub-partition-granularity run window is rejected with a diagnostic naming the model's
   partition granularity and spelling out the coarsened run window that would be accepted.
   Closes "The sub-`g_part` rejection does not yet name the coarsened window."
3. `smelt.metric()` in a partition-grain body refuses ahead of execution with
   `PartitionGrainForbidsMetrics` (classifier + diagnostic + fixture). Closes "The
   `PartitionGrainForbidsMetrics` refusal is unimplemented."
4. The four pre-outcome tracking plans (`20260530-thread-fn-registry-classification`,
   `20260616-smelt-feedback-fixes`, `20260509-meta-language-overall`,
   `20260704-model-updates-l4-batched`) are audited against the repo before re-implementation,
   so already-landed work isn't redone.
5. The `NotDerivable` lookback gate and the window-function batch-safety check classify through
   `smelt.define` bodies, matching expansion-then-analysis.
6. A CTE alias that fails to project `event_time_column` is caught by
   `EventTimeColumnNotVisibleAtOuterSelect` before execution.
7. Generator-emitted models (`ModelDef`) support the per-model overrides the declared surface
   requires.
8. A monotone-integer `partition_column` model runs first-run, backfill, and steady-state
   end-to-end (chunking, scan-filter injection, explain clamp rendering).
9. `smelt explain --json` resolves the run-relative scan window given a concrete run window;
   editor hover on a `smelt.<path>` reference shows the same clamp.
10. A `partition_column` rename gets a named refusal diagnostic and a fixture.
11. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes;
    all standing gates green (`maintenance_conformance`, `statement_parity`, `walk_coverage`
    included).

## Out of scope

- The keyed side of the determinism scope (narrowing `KeyedForbidsNondeterministic`) and all
  deletion/retention work — `20260816-keyed-grain-residue-v2`.
- Specifying the metric-expansion × time-filter composition — deliberately unspecified until
  metrics work resumes (`incremental_shapes.md` §Future Extensions); this outcome only builds
  the refusal.
- Non-deterministic row-set membership/grouping — always rejected (frozen-membership design is
  Future Extensions territory).
- Per-column `data_latency` (its own divergence, unowned here unless the phase-4 audit finds it
  cheap alongside the plan sweep — record either way in the decision log).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Audit the four cited pre-outcome tracking plans against current repo state; confirm landed vs. open; reshape later phases accordingly | pending |
| 2 | Remove compile-time pinning; volatile clocks execute as-is (transformer + tests) | pending |
| 3 | Determinism-verdict wiring: conformance-oracle column exemption, explain guarantee-ledger exemption line, suppression/diff technique gating | pending |
| 4 | Sub-`g_part` rejection names the coarsened acceptable window | pending |
| 5 | `PartitionGrainForbidsMetrics`: classifier, diagnostic, fixture | pending |
| 6 | Registry-threaded classification: lookback gate + window-function batch-safety read through `smelt.define` bodies | pending |
| 7 | CTE-only `event_time_column` detection in the outer-visibility check | pending |
| 8 | Per-`ModelDef` overrides for generator-emitted models | pending |
| 9 | Monotone-integer `partition_column` end-to-end (chunking, scan-filter injection, explain clamp) | pending |
| 10 | Clamp observability: run-relative scan window in `explain --json`; editor hover | pending |
| 11 | `partition_column` rename: refusal diagnostic + fixture | pending |
| 12 | docs-site updates (determinism scope, run-window suggestion); validate + close out (`/smelt:validate incremental_shapes`, full gate sweep) | pending |

## Decision log

- **Inherited (2026-08-16, decision track).** All product calls this outcome implements are
  recorded in `docs/research/20260816-open-questions-triage.md` and already landed as spec text
  (PR #167); this outcome makes no product decisions of its own.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
