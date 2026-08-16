# Outcome: Close the partition grain's stale-plan-tracked implementation residues

**Created:** 2026-08-15
**Status:** superseded
**Superseded by:** the delta-signature closure programme — `docs/handoffs/2026-08-16-delta-signature-closure-programme.md`. This outcome will never be run as written; its content remains reusable by the successor outcomes named there.
**Source:** `docs/specs/incremental_shapes.md` §"The partition grain" §Known Divergences;
`docs/plans/20260530-thread-fn-registry-classification.md`,
`docs/plans/20260616-smelt-feedback-fixes.md`, `docs/plans/20260509-meta-language-overall.md`,
`docs/plans/20260704-model-updates-l4-batched.md`;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`

## The outcome

Every partition-grain Known Divergences bullet that predates `docs/outcomes/` — most of them
citing a `docs/plans/*` tracker whose current status was never re-checked — either lands for real
or is confirmed already-landed by its cited plan. Function-registry-threaded classification means
the `NotDerivable` lookback-refusal gate and the window-function batch-safety check both read
through `smelt.define` function bodies, not only the outer SQL text. A CTE alias that fails to
project `event_time_column` is caught by the outer-visibility check before execution, not at
runtime. Generator-emitted models (`ModelDef`) gain the per-model overrides the spec's closed
field set currently omits. A monotone-integer `partition_column` gets a true end-to-end run path —
backfill chunking, scan-filter injection, and the `smelt explain` clamp rendering all handle a
non-date type, not only date-typed grids. Per-source clamp observability finishes: `smelt explain
--json` resolves the run-relative scan window when a concrete run window is supplied, and the
editor-hover readout ships. A `partition_column` rename gets a real refusal diagnostic with a
fixture.

## Success criteria (checkable)

1. Each of the four pre-outcome tracking plans (`20260530-thread-fn-registry-classification`,
   `20260616-smelt-feedback-fixes`, `20260509-meta-language-overall`,
   `20260704-model-updates-l4-batched`) is audited against the repo's current state before any
   re-implementation, so already-landed work isn't redone.
2. The `NotDerivable` lookback gate and the window-function batch-safety check both classify
   through `smelt.define` bodies, matching what expansion-then-analysis already promises
   elsewhere in the spec.
3. A CTE alias that fails to project `event_time_column` is caught by
   `EventTimeColumnNotVisibleAtOuterSelect` before execution.
4. Generator-emitted models (`ModelDef`) support the per-model overrides the spec's declared
   surface requires.
5. A monotone-integer `partition_column` model runs first-run, backfill, and steady-state
   end-to-end with correct chunking, scan-filter injection, and explain-clamp rendering.
6. `smelt explain --json`'s per-cell `source_bounds` resolves the run-relative scan window given a
   concrete run window; editor hover on a `smelt.<path>` reference shows the same clamp.
7. A `partition_column` rename gets a named diagnostic and a fixture exercising the refusal path.
8. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes; all
   standing gates green.

## Out of scope

- The `smelt.metric()` × time-filter-injection interaction is explicitly named "unspecified" (not
  merely unimplemented) by `incremental_shapes.md` — deciding what it should do is a design call,
  not an implementation gap against already-decided text. It stays in
  `docs/outcomes/20260815-definition-delta-migrate` §"Out of scope" pending sign-off.
- Otherwise none — this outcome exists specifically because these bullets had no other live owner
  (`docs/outcomes/20260815-definition-delta-migrate`'s scope statement). If the phase-1 audit
  finds a bullet is genuinely still owned by a live, actively-progressing plan outside
  `docs/outcomes/`, record that finding in the decision log rather than silently dropping the
  bullet from this outcome's phases.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Audit the four cited pre-outcome tracking plans against current repo state; confirm what's already landed vs. still open | pending |
| 2 | Function-registry-threaded classification: lookback gate + window-function batch-safety read through `smelt.define` bodies | pending |
| 3 | CTE-only `event_time_column` detection in the outer-visibility check | pending |
| 4 | Per-`ModelDef` overrides for generator-emitted models | pending |
| 5 | Monotone-integer `partition_column` end-to-end (backfill chunking, scan-filter injection, explain clamp) | pending |
| 6 | Per-source clamp observability: run-relative scan window in `explain --json`; editor hover | pending |
| 7 | `partition_column` rename: refusal diagnostic + fixture | pending |
| 8 | Validate + close out: `/smelt:validate incremental_shapes` clean, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
