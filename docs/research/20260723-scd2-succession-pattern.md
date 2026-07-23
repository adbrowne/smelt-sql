# SCD2 as a recognized succession pattern

**Date**: 2026-07-23
**Status**: design sketch — not scheduled. Records the machinery that smelt-maintained
incremental SCD2 would need, so the option stays concrete after the removal of the declared
`versioning: interval` profile from `docs/specs/incremental_models.md`.

## Context

The spec previously carried a declared SCD2 profile: `versioning: interval` on a keyed model,
with smelt-managed hidden validity columns (`valid_from`/`valid_to`/`is_current`), a
tracked-attribute change detector, and a bespoke "end-state equivalence (interval-keyed)"
contract. It was removed (2026-07-23) in favour of plain SQL:

```sql
SELECT
    customer_id,
    tier,
    region,
    effective_ts AS valid_from,
    LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) AS valid_to,
    LEAD(effective_ts) OVER (PARTITION BY customer_id ORDER BY effective_ts) IS NULL AS is_current
FROM smelt.customer_changes
```

Over a change stream, this window query *is* SCD2. It runs today under `refresh: full`
(rebuild each run) or `refresh: materialized_view` (engine-maintained). What was lost in the
removal is a **smelt-maintained incremental** route. This note records what that route would
require if demand materialises — as *derivation from the SQL* (pattern recognition), never as a
reintroduced declaration.

## The pattern and the theorem

**The pattern (keyed succession).** Every window function in the projection is `LEAD(t)` — or a
scalar expression over it, such as `IS NULL` — with `PARTITION BY k ORDER BY t`, where `t` is
the driving source's event-time column (a monotone clock per `timeseries.md`) and `k` is the
entity key; every other projected column is row-local. The derived output identity is `(k, t)`.

**The maintenance theorem (bounded footprint).** When a batch of new events is processed, the
only stored rows whose output changes are:

1. the new rows themselves (inserted), and
2. each new row's **immediate predecessor within its key** — the stored row whose `LEAD(t)`
   acquires (or changes) a value.

Both are locatable from stored state. Crucially the theorem covers **late events**: an event
that splices into the middle of a key's history touches exactly its predecessor (whose
`valid_to` shrinks to the new event's time) and takes its own `valid_to` from its successor
(the minimum stored `t` for the key above the new event's `t`). Late arrivals are therefore
*safe*, not forbidden — stronger than the removed profile, whose end-state equivalence leaned
on windows applying in temporal order.

**Equivalence is the standard invariant.** `LEAD` over the processed input set is exactly what
both a full rebuild and a correctly-maintained incremental table compute, so
`incremental_state(S) == full_refresh(inputs ∈ S)` applies directly. No bespoke "interval-keyed
specialisation" is needed — a real simplification over the removed profile, which required its
own equivalence definition precisely because the user's SELECT did not define the output
(smelt's hidden combiner did).

## Machinery per layer

- **`model_properties.md` — a keyed-succession verdict.** A new property produced by the shared
  bottom-up walk (per the Property composition walk rule, a leaf classifier the walk invokes —
  never an ad hoc SQL-text scan): all window functions in the projection are succession
  functions over the monotone clock within key partitions; everything else row-local. This is
  the genuinely new analysis — the walk's current vocabulary is aggregate combiners; window
  functions bring ORDER BY semantics it has no machinery for.
- **`incremental_models.md` — one admission-corner instance.** The per-cell obligations
  instantiate cleanly: replayable input = the append-only change stream; combiner = the
  succession patch (below); bounded reach = the run's event-time window on the driving source;
  bounded footprint = the theorem above; well-defined groups = the LEAD-derived columns
  (`valid_to`, `is_current`) form one column group, the row-local columns another (write-once).
- **`model_transforms.md` — a succession-patch technique.** A keyed `MERGE` inserting the new
  rows plus a targeted update of each predecessor row's LEAD-derived columns. Emitted by a pure
  emitter in `smelt-logical`'s maintenance layer (statement-emission single-owner rule); covered
  by `statement_parity` and by `maintenance_conformance` recipes that must include late-event
  and delete-event sequences — the generative gate is where the splice claim earns trust.

## The hard parts

1. **Delete events.** A CDC delete should close a version without opening one. In SQL that is
   `LEAD` computed *before* filtering out delete rows (their timestamp must still feed the
   predecessor's `valid_to`), then `WHERE NOT is_delete` *after* the window. The classifier must
   admit that filter-after-window shape — recognisable, but it widens the pattern grammar
   beyond a single projection shape.
2. **Pattern-grammar boundaries.** Every admissible variation (expressions over `LEAD`,
   multiple succession columns, post-window filters) is a classifier case, and everything
   outside the grammar needs a precise fail-loud diagnostic explaining *why* the model fell out
   of the pattern. The diagnostic-quality work is the long tail.

## What stays out regardless

**SCD2 over mutable snapshots.** Deriving a change stream by diffing successive snapshot scans
would stamp version boundaries with the scan time — the run clock — making the history depend
on when `smelt build` happened to run, which violates replay-safety and the equivalence
invariant. A snapshot-to-change-stream facility, if ever wanted, is a *source-layer* concern
(`sources.md`), not part of this pattern.

## Why derivation beats the removed declaration

The machinery is not SCD2-specific: the succession theorem maintains *any* `LEAD`/`LAG`-over-
clock-within-key model — next-event features, sessionisation gaps, inter-event durations.
`versioning: interval` bought one feature; the classifier buys a family. It also dissolves the
removed profile's open questions (hidden validity-column surface, NULL vs far-future sentinel,
tracked-attribute selection): the user's SQL states all of it explicitly. The work is justified
by demand for the family, not by SCD2 alone.

**Size estimate**: comparable to the key grain's combiner machinery but narrower — one
classifier, one admission instance, one technique, plus conformance recipes.

## References

- `docs/specs/incremental_models.md` §Limitations (the current posture), §Future Extensions
  (cites this note)
- `docs/specs/materialized_view.md` §Design "No named pattern" (the engine-maintained SCD2 route)
- `docs/research/20260522-cumulative-as-its-own-rule.md` — the earlier sibling-rule sketch
  (`scd2`) this supersedes in approach (recognition over declaration)
- `docs/research/20260703-model-updates.md` Part 17 — the removed declared surface's design
  history
