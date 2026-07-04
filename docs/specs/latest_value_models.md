---
feature: latest_value_models
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Latest-Value Refresh Mode (SCD Type 1)

> **What this is.** A normative spec for `refresh: latest_value` — a smelt-owned keyed-output refresh mode that keeps **only the current row** per key, overwriting the prior value in place. It is the Type-1 slowly-changing-dimension pattern, named without the vendor "SCD" jargon. Covers the frontmatter selector, the one-row-per-key output shape, the end-state equivalence contract, and how it relates to its siblings. Out of scope: the version-keeping mode (`versioned_models.md`); the running-aggregate mode (`cumulative_aggregate.md`); engine-owned maintenance (`materialized_view.md`); the batched mode (`batched_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** The mode is specified ahead of implementation; `refresh: latest_value` currently produces an unknown-refresh-value error. Delivered by a phase of `docs/plans/20260704-model-updates.md`.

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: latest_value
---

SELECT
    customer_id,          -- the natural key
    tier,
    region
FROM smelt.customers_snapshot
```

`refresh: latest_value` is the entire opt-in; it implies a stored `table` (`models.md` §Design). It **forbids** a `timeseries:` block and a `batched:` block *on the model itself* — the output is a keyed lookup, not a partitioned table (`models.md` §"Constraint violations"). This forbids output partitioning, not event-time-aware consumption: like `cumulative`, a latest-value model over a source that carries a `timeseries:` declaration consumes that source window-forward (see §Semantics).

The model's SELECT projects the **natural key** and the attribute columns. smelt maintains one row per key: each `smelt build` upserts the incoming rows over the stored ones, overwriting changed attributes. No history is kept — a changed value replaces the old one with no trace.

### Output shape

One row per natural key: the projected columns, always reflecting the most recently processed value. No validity columns, no version rows (that is `versioned`).

## Semantics

### End-state equivalence

`refresh: latest_value` upholds the keyed end-state contract (`cumulative_aggregate.md` §"Cross-partition equivalence"): the stored row for each key equals what a full rebuild would produce — the last-writer value over the processed inputs. Order-independence holds up to the definition of "latest": when the source carries an ordering column (an updated-at), the per-key combiner is max-by-ordering-key — a commutative monoid — so the retained value is independent of merge order, and run windows may be applied out of order or backfilled in slices. Absent an ordering column, "latest" is last-processed: order-dependent (not a monoid), which forces strictly sequential window application (the derived-ordering posture of `docs/research/20260703-model-updates.md` §19.4). smelt owns freshness (pull) — correct as of the last `smelt build`.

### Input consumption is derived from the source

How new input is discovered is never declared on the model; it follows from the source's shape (`docs/research/20260703-model-updates.md` Part 19):

- **Window-forward** — a source carrying a `timeseries:` declaration is consumed in `--event-time` run windows applied to the *source's* `partition_column`, exactly as `cumulative` consumes its driving source (`cumulative_aggregate.md` §CLI). Only the new tail is read.
- **Snapshot-diff** — a mutable snapshot source (no monotone clock) is re-scanned each run and upserted whole; the end-state contract is identical, only the scan cost differs.

## Design

**Named `latest_value`, symmetric with `versioned`.** The pair reads as the Type-1 ↔ Type-2 contrast — overwrite-and-keep-current vs keep-every-version — without either name mentioning "slowly-changing dimension" (`docs/research/20260703-model-updates.md` §17.4). `latest_value` says exactly what the stored table holds.

**A smelt-owned pattern, distinct from engine-owned overwrite.** `latest_value` is a pattern smelt maintains itself (it owns the combiner: upsert-overwrite). An engine-maintained equivalent is hand-written SQL under `refresh: materialized_view`, not a `latest_value` + maintainer flag (`materialized_view.md` §Design "No named pattern").

**Why a distinct mode from `versioned` rather than a `history: false` knob on it.** Keeping-every-version and keeping-only-current are different end-state contracts and different output shapes (interval-keyed vs one-row-per-key). Collapsing them under one mode with a boolean would put two contracts behind one name — the strategy-sub-knob footgun the refresh enum exists to avoid (`models.md` §Design "Refresh modes are peers"). They are peers.

## Constraints & Invariants

1. **`refresh: latest_value` implies `table` storage.** No `materialization:` restatement.
2. **No `timeseries:` and no `batched:` block on the model itself.** Keyed output; not a partitioned batched build. Window-forward consumption of a `timeseries:` *source* is derived and in-bounds (§Semantics).
3. **Exactly one row per natural key.** The upsert overwrites; no version rows accumulate.
4. **End-state equivalent.** The stored value equals the last-writer value over processed inputs.

## Known Divergences / Open Questions

- **Not implemented.** Declaring `refresh: latest_value` currently errors. The classifier and the upsert-overwrite execution (via `merge_into`) are delivered by `docs/plans/20260704-model-updates.md`.
- **Definition of "latest" is unsettled, with a preferred direction.** Whether the retained value is chosen by a source ordering column (an updated-at, derived from the SQL) or simply last-processed remains to be settled when the mode is built — but the algebra prefers the ordering column: it makes the combiner a commutative monoid (order-independent merges, parallel/out-of-order backfill), while last-processed is order-dependent and forces sequential windows (`docs/research/20260703-model-updates.md` §19.4). Prefer deriving the ordering column from the SQL over a strategy block; treat last-processed as the derived-ordered fallback. How ties on the ordering key break deterministically is part of the same question.
- **Deletions.** Whether a key vanishing from the incoming set is deleted from the store or retained is an Open Question, shared with `versioned` (`docs/research/20260703-model-updates.md` §18.2).

## References

- **Related specs**:
  - [`models.md`](models.md) — the refresh axis; the input-consumption axis (canonical home for window-forward vs snapshot-diff derivation); `latest_value` as a keyed-output peer
  - [`versioned_models.md`](versioned_models.md) — the symmetric Type-2 mode (keep every version)
  - [`cumulative_aggregate.md`](cumulative_aggregate.md) — the running-aggregate keyed mode; source of the keyed end-state contract and `merge_into` execution model
  - [`materialized_view.md`](materialized_view.md) — engine-owned maintenance (where hand-written overwrite SQL goes instead)
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming); Part 19 (the input-consumption axis; the ordering-column monoid argument)
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
