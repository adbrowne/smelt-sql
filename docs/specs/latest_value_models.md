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

`refresh: latest_value` is the entire opt-in; it implies a stored `table` (`models.md` §Design). It **forbids** a `timeseries:` block and a `batched:` block — the output is a keyed lookup, not a partitioned table (`models.md` §"Constraint violations").

The model's SELECT projects the **natural key** and the attribute columns. smelt maintains one row per key: each `smelt build` upserts the incoming rows over the stored ones, overwriting changed attributes. No history is kept — a changed value replaces the old one with no trace.

### Output shape

One row per natural key: the projected columns, always reflecting the most recently processed value. No validity columns, no version rows (that is `versioned`).

## Semantics

### End-state equivalence

`refresh: latest_value` upholds the keyed end-state contract (`cumulative_aggregate.md` §"Cross-partition equivalence"): the stored row for each key equals what a full rebuild would produce — the last-writer value over the processed inputs. Order-independence holds up to the definition of "latest": when the source carries an ordering column (an updated-at), the retained value is the one with the maximal ordering key regardless of merge order; absent an ordering column, "latest" is last-processed, which is a declared/derived choice (Open Question). smelt owns freshness (pull) — correct as of the last `smelt build`.

## Design

**Named `latest_value`, symmetric with `versioned`.** The pair reads as the Type-1 ↔ Type-2 contrast — overwrite-and-keep-current vs keep-every-version — without either name mentioning "slowly-changing dimension" (`docs/research/20260703-model-updates.md` §17.4). `latest_value` says exactly what the stored table holds.

**A smelt-owned pattern, distinct from engine-owned overwrite.** `latest_value` is a pattern smelt maintains itself (it owns the combiner: upsert-overwrite). An engine-maintained equivalent is hand-written SQL under `refresh: materialized_view`, not a `latest_value` + maintainer flag (`materialized_view.md` §Design "No named pattern").

**Why a distinct mode from `versioned` rather than a `history: false` knob on it.** Keeping-every-version and keeping-only-current are different end-state contracts and different output shapes (interval-keyed vs one-row-per-key). Collapsing them under one mode with a boolean would put two contracts behind one name — the strategy-sub-knob footgun the refresh enum exists to avoid (`models.md` §Design "Refresh modes are peers"). They are peers.

## Constraints & Invariants

1. **`refresh: latest_value` implies `table` storage.** No `materialization:` restatement.
2. **No `timeseries:` and no `batched:` block.** Keyed output; not a partitioned batched build.
3. **Exactly one row per natural key.** The upsert overwrites; no version rows accumulate.
4. **End-state equivalent.** The stored value equals the last-writer value over processed inputs.

## Known Divergences / Open Questions

- **Not implemented.** Declaring `refresh: latest_value` currently errors. The classifier and the upsert-overwrite execution (via `merge_into`) are delivered by `docs/plans/20260704-model-updates.md`.
- **Definition of "latest" is unsettled.** Whether the retained value is chosen by a source ordering column (an updated-at, derived from the SQL) or simply last-processed, and how that is declared or derived, is an Open Question to settle when the mode is built. Prefer deriving from the SQL over a strategy block.
- **Deletions.** Whether a key vanishing from the incoming set is deleted from the store or retained is an Open Question, shared with `versioned` (`docs/research/20260703-model-updates.md` §18.2).

## References

- **Related specs**:
  - [`models.md`](models.md) — the refresh axis; `latest_value` as a keyed-output peer
  - [`versioned_models.md`](versioned_models.md) — the symmetric Type-2 mode (keep every version)
  - [`cumulative_aggregate.md`](cumulative_aggregate.md) — the running-aggregate keyed mode; source of the keyed end-state contract and `merge_into` execution model
  - [`materialized_view.md`](materialized_view.md) — engine-owned maintenance (where hand-written overwrite SQL goes instead)
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming)
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
