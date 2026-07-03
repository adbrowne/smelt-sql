---
feature: versioned_models
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Versioned Refresh Mode (SCD Type 2)

> **What this is.** A normative spec for `refresh: versioned` — a smelt-owned keyed-output refresh mode that keeps **every version** of a key, each stamped with a validity interval. It is the Type-2 slowly-changing-dimension pattern, named without the vendor "SCD" jargon. Covers the frontmatter selector, the keyed-plus-interval output shape, the end-state equivalence contract, and how it relates to its siblings. Out of scope: the running-aggregate keyed mode (`cumulative_aggregate.md`); the overwrite keyed mode (`latest_value_models.md`); engine-owned maintenance (`materialized_view.md`); the batched mode (`batched_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** The mode is specified ahead of implementation; `refresh: versioned` currently produces an unknown-refresh-value error. Delivered by a phase of `docs/plans/20260704-model-updates.md`.

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: versioned
---

SELECT
    customer_id,          -- the natural key
    tier,
    region
FROM smelt.customers_snapshot
```

`refresh: versioned` is the entire opt-in; it implies a stored `table` (`models.md` §Design). It **forbids** a `timeseries:` block and a `batched:` block — the output is a keyed lookup, not a partitioned table (`models.md` §"Constraint violations").

The model's SELECT projects the **natural key** and the tracked attribute columns as they are *now*. smelt maintains the version history: each `smelt build` compares the incoming rows against the stored current version per key and, where a tracked attribute changed, closes the prior version and opens a new one.

### Output shape

The stored table carries the projected columns plus smelt-managed validity columns: a `valid_from` / `valid_to` interval and an `is_current` flag (exact column names are an Open Question). A key with three successive states yields three rows — two closed intervals and one open (`valid_to` NULL / sentinel, `is_current = true`).

## Semantics

### End-state equivalence (interval-keyed)

`refresh: versioned` upholds the keyed end-state contract (`cumulative_aggregate.md` §"Cross-partition equivalence"), specialised to intervals: the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same sequence of processed snapshots, independent of the order in which non-overlapping snapshots were merged. smelt owns freshness (pull) — the history is correct as of the last `smelt build`.

### Change detection

A new version is opened for a key only when a **tracked attribute** changes between the stored current version and the incoming row. Which columns are tracked (all projected non-key columns by default vs an explicit subset) is an Open Question. A key present in the store but absent from the incoming set, and late-arriving corrections to a closed interval, are Open Questions (see below).

## Design

**Named `versioned`, not `scd2`.** The pair `versioned` (keep every version with a validity interval) / `latest_value` (keep only the current row) is deliberately symmetric and reads as the Type-2 ↔ Type-1 contrast without either name mentioning "slowly-changing dimension." Vendor jargon in a refresh value was rejected: the enum values name *what the mode does to your data*, legibly, not a modelling-methodology acronym (`docs/research/20260703-model-updates.md` §17.4).

**A smelt-owned pattern, distinct from engine-owned SCD.** `versioned` is one of the patterns smelt maintains itself (it owns the combiner: close-old / open-new). An *engine-maintained* SCD2 is not a variant of this mode — it is hand-written SCD2 SQL declared `refresh: materialized_view`, where the engine's IVM runtime does the maintenance (`materialized_view.md` §Design "No named pattern"). The two are not `versioned` + a maintainer flag; they are different modes with different freshness owners (`docs/research/20260703-model-updates.md` §17.8).

**Derive from SQL where possible.** Following the keyed-mode posture, the natural key and tracked attributes should be derived from the SQL and the model's declared key rather than restated in a strategy block wherever that is unambiguous (`cumulative_aggregate.md` §Design). The precise derive-vs-declare line for change-tracking columns is an Open Question.

## Constraints & Invariants

1. **`refresh: versioned` implies `table` storage.** No `materialization:` restatement.
2. **No `timeseries:` and no `batched:` block.** Keyed + interval output; not a partitioned batched build.
3. **Validity intervals are non-overlapping per key.** At most one open (`is_current`) version per key at any time.
4. **End-state equivalent and order-independent.** Merging non-overlapping snapshots in any order converges to the same version history.

## Known Divergences / Open Questions

- **Not implemented.** Declaring `refresh: versioned` currently errors. The classifier, the version-maintenance execution (close-old / open-new via `merge_into`), and the validity-column management are delivered by `docs/plans/20260704-model-updates.md`.
- **Validity-column surface is unsettled.** Exact names/types of `valid_from` / `valid_to` / `is_current`, whether the open interval uses NULL or a sentinel far-future timestamp, and whether these are configurable are Open Questions to settle when the mode is built.
- **Tracked-attribute selection is unsettled.** All projected non-key columns vs an explicitly declared subset; how a modeller marks a column as untracked (e.g. a slowly-drifting field that should not open a new version). Prefer deriving from SQL over a strategy block; the exact line is undecided.
- **Deletions and late corrections.** Whether a key vanishing from the incoming set closes its current version, and how a correction to an already-closed interval is applied, need their own design — the same retraction question the keyed modes share (`cumulative_aggregate.md` §"Reprocessing semantics"; `docs/research/20260703-model-updates.md` §18.2).
- **Umbrella subsumption.** Whether `versioned` shares execution machinery with the other keyed modes or is a standalone rule is settled here as **standalone** (its own spec, its own classifier), consistent with the narrow-composable-rules posture (`docs/research/20260522-cumulative-as-its-own-rule.md`).

## References

- **Related specs**:
  - [`models.md`](models.md) — the refresh axis; `versioned` as a keyed-output peer
  - [`latest_value_models.md`](latest_value_models.md) — the symmetric Type-1 mode (overwrite, keep current)
  - [`cumulative_aggregate.md`](cumulative_aggregate.md) — the running-aggregate keyed mode; source of the keyed end-state contract and `merge_into` execution model
  - [`materialized_view.md`](materialized_view.md) — engine-owned maintenance (where hand-written SCD2 SQL goes instead)
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming)
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches (`scd2`, `latest_value`, `accumulating_snapshot`)
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
