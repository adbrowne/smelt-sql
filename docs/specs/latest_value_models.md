---
feature: latest_value_models
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Latest-Value Refresh Mode (SCD Type 1)

> **What this is.** A normative spec for `refresh: latest_value` — a smelt-owned keyed-output refresh mode that keeps **only the current row** per key, overwriting the prior value in place. It is the Type-1 slowly-changing-dimension pattern, named without the vendor "SCD" jargon. This spec is a **composition** in the sense of `model_maintenance.md`: it presents the composition table for the mode and defines only the mode's **local** machinery — the upsert-overwrite combiner, the definition of "latest", its classifier, and its input-consumption derivation. It references shared capabilities (the equivalence invariant, the algebraic ladder, the algebraic discriminants, keyed `merge_into`, the windowed-keyed-maintenance driver) **by name** and never re-specifies them. Out of scope, with their own homes: the equivalence invariant and ladder (`model_maintenance.md`); the discriminants and driving-fact resolution (`model_properties.md`); `merge_into` and the driver (`model_transforms.md`); the refresh axis and input-consumption axis (`models.md`); the version-keeping mode (`versioned_models.md`); the running-aggregate mode (`cumulative_aggregate.md`); engine-owned maintenance (`materialized_view.md`); the batched mode (`batched_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** `refresh: latest_value` does not parse yet (see §Known Divergences). Delivered by a phase of `docs/plans/20260704-model-updates.md`.

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

### Composition table

Per the composition contract (`model_maintenance.md` §"The composition contract"), `refresh: latest_value` composes as:

| Facet | This mode |
|---|---|
| **Properties required** | The **value-monotone vs order-monotone** discriminant resolving to the order-monotone `MAX_BY`-by-key case (`model_properties.md` §"Algebraic discriminants") — the retained value is a per-key semilattice fold whose presented row may switch as a later value arrives. **Driving-fact / anchor resolution** (`model_properties.md`) when the source is joined. **Input-delta discovery** to derive how new rows are found. |
| **World-facts consumed** | The **timeseries clock** of a clocked driving source (`event_time_column`/`partition_column`, `timeseries.md`) for window-forward consumption; **or** a mutable snapshot source's **mutation profile** (`sources.md`) for snapshot-diff consumption. |
| **Transform driven** | Keyed **`merge_into`** (target-as-replica) sequenced by the **windowed-keyed-maintenance driver** (`model_transforms.md`), realising the mode's local **upsert-overwrite** combiner (below). |
| **Output shape** | **Keyed** — one current row per natural key (`models.md` §"Refresh axis"). |

## Semantics

### End-state equivalence

`refresh: latest_value` upholds the **end-state equivalence invariant** for keyed output (`model_maintenance.md` §"The equivalence invariant"): for any set of processed source partitions and any ordering over them, the stored row per key equals `full_refresh` restricted to those inputs — the last-writer value. Order-independence holds up to the **definition of "latest"** (below): with an ordering column the combiner is a commutative monoid, so run windows may be applied out of order or backfilled in slices; without one it is order-dependent and forces sequential application. smelt owns freshness (pull) — correct as of the last `smelt build`.

### Upsert-overwrite (the local combiner)

The stored table *is* the keyed state, one row per key; `latest_value`'s combiner folds each incoming row over the stored one by **overwriting** it. This is the mode-local realisation of the `merge_into` transform — the "upsert-overwrite" combiner catalogued as staying in this mode spec (`model_transforms.md` §"Transforms that stay in a mode spec"). Its algebra is fixed by the definition of "latest":

- **Max-by-ordering monoid.** When the source carries an ordering column (an updated-at), the per-key combiner is **max-by-ordering-key** — a commutative, associative, idempotent semilattice fold (identity = the empty partition). It is order-monotone, not value-monotone: the *ordering key* moves one way, but the *presented attribute row* may switch to any value the winning row carries. Because it is a monoid, `merge_into` folds a keyed delta into the store with matched keys overwritten and unmatched inserted, and the retained row is independent of merge order — the driver may step partitions in any order and backfill in slices.
- **Last-processed fallback.** Absent an ordering column, "latest" is **last-processed**: the combiner keeps whichever row the current run wrote last. This is order-dependent — not a monoid — so window application must be strictly sequential; out-of-order or parallel backfill is refused. This is the derived-ordered posture, the honest fallback when no ordering column can be derived.

Either way the invariant discharges: the stored value is the last-writer value over the processed inputs. The distinction is only *which* representation is order-independent — it never changes what the stored relation means.

### Definition of "latest" and the preferred direction

"Latest" is defined by an **ordering column derived from the SQL** (an updated-at / version column the projection carries), not declared in a strategy block. The algebra **prefers** the ordering column: it makes the combiner the max-by-ordering monoid above (order-independent merges, parallel/out-of-order backfill), whereas last-processed is order-dependent and sequential. Prefer deriving the ordering column from the model's SQL; treat last-processed as the derived-ordered fallback. How ties on the ordering key break deterministically is part of the same open question (see §Known Divergences).

### The classifier

A model is admitted to `latest_value` when its body classifies as an **order-monotone keyed overwrite**: the projection carries a natural key and a set of attribute columns, and the per-key fold is a `MAX_BY`-shaped semilattice (ordering column present) or a last-processed overwrite (ordering column absent). The classifier reads the value/order-monotone discriminant and the driving-fact resolution from `model_properties.md`; it is fail-closed per the validator-not-chooser rule (`model_maintenance.md` §"Validator, not chooser") — an ambiguous key set, a non-overwrite fold, or an unresolvable anchor is rejected with a diagnostic, never silently downgraded to full refresh.

### Input consumption is derived from the source

How new input is discovered is never declared on the model; it is the mode-local application of the input-consumption axis (`models.md` §"Input-consumption axis"), derived from the source's shape:

- **Window-forward** — a source carrying a `timeseries:` declaration is consumed in `--event-time` run windows applied to the *source's* `partition_column`, exactly as `cumulative` consumes its driving source (`cumulative_aggregate.md` §CLI). Only the new tail is read.
- **Snapshot-diff** — a mutable snapshot source (no monotone clock) is re-scanned each run and upserted whole; the end-state contract is identical, only the scan cost differs.

## Design

**Named `latest_value`, symmetric with `versioned`.** The pair reads as the Type-1 ↔ Type-2 contrast — overwrite-and-keep-current vs keep-every-version — without either name mentioning "slowly-changing dimension" (`docs/research/20260703-model-updates.md` §17.4). `latest_value` says exactly what the stored table holds.

**A smelt-owned pattern, distinct from engine-owned overwrite.** `latest_value` is a pattern smelt maintains itself (it owns the combiner: upsert-overwrite). An engine-maintained equivalent is hand-written SQL under `refresh: materialized_view`, not a `latest_value` + maintainer flag (`materialized_view.md` §Design "No named pattern").

**Why a distinct mode from `versioned` rather than a `history: false` knob on it.** Keeping-every-version and keeping-only-current are different end-state contracts and different output shapes (interval-keyed vs one-row-per-key). Collapsing them under one mode with a boolean would put two contracts behind one name — the strategy-sub-knob footgun the refresh enum exists to avoid (`models.md` §Design "Refresh modes are peers"). They are peers.

**Order-monotone, not a fifth ladder rung.** `latest_value` sits on the monoid rungs of the algebraic ladder (`model_maintenance.md`): the max-by-ordering combiner is a semilattice, maintainable by a plain `merge_into` loop with no retraction. Its distinguishing algebra is captured by the order-monotone discriminant in `model_properties.md`, not by a new rung here.

## Constraints & Invariants

1. **`refresh: latest_value` implies `table` storage.** No `materialization:` restatement.
2. **No `timeseries:` and no `batched:` block on the model itself.** Keyed output; not a partitioned batched build. Window-forward consumption of a `timeseries:` *source* is derived and in-bounds (§Semantics).
3. **Exactly one row per natural key.** The upsert overwrites; no version rows accumulate.
4. **End-state equivalent** (`model_maintenance.md`). The stored value equals the last-writer value over processed inputs.
5. **Validator, not chooser** (`model_maintenance.md`). An un-classifiable model is rejected with a diagnostic, never downgraded silently.

## Known Divergences / Open Questions

- **Not implemented — the mode does not parse.** `refresh: latest_value` is currently rejected at config deserialization: `RefreshStrategy` (`crates/smelt-core/src/config.rs`) accepts only `full`, `batched`, `cumulative`, and `materialized_view`, so `latest_value` produces an *Invalid refresh strategy* (unknown-refresh-value) error before any classifier runs. Adding the enum variant, the order-monotone classifier, and the upsert-overwrite execution (via `merge_into` + the driver) is delivered by `docs/plans/20260704-model-updates.md`.
- **Definition of "latest" is unsettled, with a preferred direction.** Whether the retained value is chosen by a source ordering column (an updated-at, derived from the SQL) or simply last-processed remains to be settled — but the algebra prefers the ordering column (order-independent monoid merges) over last-processed (order-dependent, sequential), per `docs/research/20260703-model-updates.md` §19.4. How ties on the ordering key break deterministically is part of the same question.
- **Deletions.** Whether a key vanishing from the incoming set is deleted from the store or retained is an Open Question, shared with `versioned` (`docs/research/20260703-model-updates.md` §18.2).

## References

- **Related specs**:
  - [`model_maintenance.md`](model_maintenance.md) — the equivalence invariant, the algebraic ladder, the composition contract, validator-not-chooser
  - [`model_properties.md`](model_properties.md) — the value/order-monotone discriminant, driving-fact resolution, input-delta discovery
  - [`model_transforms.md`](model_transforms.md) — keyed `merge_into`, the windowed-keyed-maintenance driver, source-filter pushdown; upsert-overwrite listed as staying in this mode spec
  - [`models.md`](models.md) — the refresh axis, the input-consumption axis, the three-state declaration law; `latest_value` as a keyed-output peer
  - [`versioned_models.md`](versioned_models.md) — the symmetric Type-2 mode (keep every version)
  - [`cumulative_aggregate.md`](cumulative_aggregate.md) — the running-aggregate keyed mode; reference implementation of the keyed-maintenance path
  - [`materialized_view.md`](materialized_view.md) — engine-owned maintenance (where hand-written overwrite SQL goes instead)
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming); Part 19 (the input-consumption axis; the ordering-column monoid argument)
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance-framework design this spec composes against
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
</content>
</invoke>
