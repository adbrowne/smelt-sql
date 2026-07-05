---
feature: versioned_models
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Versioned Refresh Mode (SCD Type 2)

> **What this is.** A normative spec for `refresh: versioned` — a smelt-owned keyed-output refresh mode that keeps **every version** of a key, each stamped with a validity interval. It is the Type-2 slowly-changing-dimension pattern, named without the vendor "SCD" jargon. This spec is a **composition** (`model_maintenance.md` §"The composition contract"): it presents the composition table referencing shared capabilities **by name**, then defines the versioned-**local** machinery in full — the close-old / open-new interval combiner, the smelt-managed validity columns, tracked-attribute selection, deletion handling, and event-time-stamped validity. Out of scope, owned elsewhere: the equivalence invariant and algebraic ladder (`model_maintenance.md`); the monotonicity/ordering discriminants, driving-fact resolution, and ordered-execution proofs (`model_properties.md`); keyed `merge_into`, the windowed-keyed-maintenance driver, and source-filter pushdown (`model_transforms.md`); the `refresh:` enum, the three-state declaration law, and the input-consumption axis (`models.md`); the keyed mode covering the overwrite, running-aggregate, and milestone patterns (`keyed_models.md`); engine-owned maintenance (`materialized_view.md`); the batched mode (`batched_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** The mode is specified ahead of implementation; `refresh: versioned` does not parse today (§Known Divergences). Delivered by a phase of `docs/plans/20260704-model-updates.md`.

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

`refresh: versioned` is the entire opt-in; it implies a stored `table` (`models.md` §Design). It **forbids** a `timeseries:` block and a `batched:` block *on the model itself* — the output is a keyed lookup, not a partitioned table (`models.md` §"Constraint violations"). This forbids output partitioning, not event-time-aware consumption: like `cumulative`, a versioned model over a source that carries a `timeseries:` declaration (an update-events / CDC feed) consumes that source window-forward (see §"Input consumption").

The model's SELECT projects the **natural key** and the tracked attribute columns as they are *now*. smelt maintains the version history: each `smelt build` compares incoming rows against the stored current version per key and, where a tracked attribute changed, closes the prior version and opens a new one.

### Output shape

Keyed **plus** a validity interval. The stored table carries the projected columns and the smelt-managed validity columns — a `valid_from` / `valid_to` interval and an `is_current` flag (exact names/types are an Open Question). A key with three successive states yields three rows: two closed intervals and one open (`valid_to` NULL / sentinel, `is_current = true`).

## Semantics

### Composition table

Per the composition contract (`model_maintenance.md` §"The composition contract"), `refresh: versioned` composes the capabilities below **by name**; each is owned by the spec cited. The versioned-local transform (close-old / open-new) is defined in full further down — it stays in this mode spec by the placement criterion (`model_transforms.md` §"Transforms that stay in a mode spec").

| Composition facet | Capability (owned elsewhere unless marked local) |
|---|---|
| **Invariant upheld** | End-state equivalence, interval-keyed specialisation (`model_maintenance.md` §"The equivalence invariant") |
| **Ladder rung** | Value/order-monotone keyed fold — the close-old / open-new combiner is a keyed semilattice-style step, on the smelt-maintained side of the algebraic ladder (`model_maintenance.md` §"The algebraic maintenance ladder") |
| **Properties required** | Algebraic **monotonicity / ordering discriminants** (value-monotone vs order-monotone) + **event-time monotonicity trace** + **driving-fact / anchor resolution** + **window-independence / ordered-execution** (`model_properties.md`) |
| **World-facts consumed** | The **timeseries clock** of an update-events / CDC feed (`timeseries.md`), *or* a mutable snapshot's **source mutation profile** (`sources.md`) — one of the two, derived from the source's shape, never declared on the model |
| **Transforms driven** | Keyed **`merge_into`** sequenced by the **windowed-keyed-maintenance driver**, with **source-filter pushdown** on the driving source (all `model_transforms.md`), folding through the **close-old / open-new interval maintenance** combiner (local, below) |
| **Output shape** | Keyed **+ validity interval** — one row per `(key, version)` with a non-overlapping `[valid_from, valid_to)` interval and at most one `is_current` row per key |

### End-state equivalence (interval-keyed)

`refresh: versioned` upholds the **end-state equivalence invariant** in its interval-keyed specialisation (`model_maintenance.md` §"The equivalence invariant"): the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same set of processed snapshots, independent of the order in which non-overlapping snapshots were merged. smelt owns freshness (pull) — the history is correct as of the last `smelt build`.

Order-independence holds because validity is anchored to the source's event time, not the run clock (see §"Validity stamped from source event-time"): the close-old / open-new combiner reads versions in event order via the **driving-fact / anchor resolution** and **ordered-execution** proofs (`model_properties.md`), so replays and out-of-order windows converge to the same history rather than shifting interval boundaries.

### Input consumption is derived from the source

How new input is discovered is never declared on the model; it is the input-consumption axis (`models.md` §"Input-consumption axis"), derived from the source's shape:

- **Window-forward** — a source carrying a `timeseries:` declaration (an update-events / CDC feed) is consumed in `--event-time` run windows applied to the *source's* `partition_column`, exactly as `keyed` consumes its driving source (`keyed_models.md` §CLI). Only the new tail is read (source-filter pushdown, `model_transforms.md`). Because the close-old / open-new combiner consumes versions in event order, windows are applied in temporal order (ordered execution, `model_properties.md`).
- **Snapshot-diff** — a mutable snapshot source (no monotone clock) is re-scanned each run and compared against the stored current versions; the end-state contract is identical, only the scan cost differs.

The choice between the two is the mutation-profile world-fact (`sources.md`) feeding the input-delta-discovery proof (`model_properties.md`); moving along this axis never changes the equivalence contract, only what is scanned.

## Versioned-local machinery

The following is owned in full by this spec — it is the machinery meaningful only inside `refresh: versioned`.

### Close-old / open-new interval maintenance (the combiner)

The combiner the windowed-keyed-maintenance driver folds through. For each incoming row, keyed by natural key:

1. Look up the key's current (open) version in the stored table.
2. If no current version exists, **open** a new version: insert the row with `valid_from` = the incoming event time, `valid_to` = open, `is_current = true`.
3. If a current version exists and a **tracked attribute** differs, **close** the old version (set its `valid_to` = the incoming event time, `is_current = false`) and **open** a new one at that boundary.
4. If a current version exists and no tracked attribute differs, do nothing — no spurious version.

The close and the open share the same boundary timestamp, so intervals abut without gaps or overlaps. The mechanism is emitted as a keyed `merge_into` (`model_transforms.md`) — matched keys close-and-reopen, unmatched keys open — so history is never re-read wholesale.

### Validity columns (smelt-managed)

`valid_from`, `valid_to`, and `is_current` are **managed by smelt**, not projected by the user's SELECT. The user projects only the natural key and the tracked attributes; smelt appends and maintains the interval columns. The open interval's `valid_to` is either NULL or a far-future sentinel (undecided — see §Known Divergences); `is_current` is a convenience flag equivalent to "`valid_to` is open" that indexes the current-version lookup the combiner performs every run.

### Tracked-attribute selection

A new version is opened for a key only when a **tracked attribute** changes between the stored current version and the incoming row. By default every projected non-key column is tracked. Whether a modeller can mark a column *untracked* (a slowly-drifting field that should not open a new version), and whether that is derived from the SQL or declared, is an Open Question (§Known Divergences); the posture is to derive the key and tracked set from the SQL where unambiguous rather than restate them in a strategy block (`keyed_models.md` §Design).

### Validity stamped from source event-time (not run clock)

`valid_from` / `valid_to` boundaries are stamped from the **source's event time** — the update-events feed's event-time column, or the snapshot's as-of timestamp — **never the run clock**. This is what makes the history replay-safe: re-running a window, or backfilling windows out of order, reproduces byte-identical interval boundaries, so end-state equivalence survives replays. A run-clock stamp would make the same version boundary depend on *when* `smelt build` happened to run, breaking order-independence.

### Deletion handling

A key present in the store but absent from the incoming set is a **retraction**, and how it is handled is settled here as a soft-close: the key's current version is closed (`valid_to` set, `is_current = false`) with no new version opened, marking "no longer present as of this event time." The event time used is the run's window boundary for a window-forward feed, or the snapshot's as-of time for snapshot-diff. A hard delete (physically removing the key's rows) is **not** the default — the whole point of `versioned` is to retain history — but the exact surface for opting into a hard delete, and for *late corrections* to an already-closed interval, remain Open Questions (they are the retraction question the keyed modes share; `keyed_models.md` §"Reprocessing"). A CDC feed that carries explicit delete events resolves this directly: the delete event is the close signal.

## Design

**Named `versioned`, not `scd2`.** The pair `versioned` (keep every version with a validity interval) / `latest_value` (keep only the current row) is deliberately symmetric and reads as the Type-2 ↔ Type-1 contrast without either name mentioning "slowly-changing dimension." Vendor jargon in a refresh value was rejected: the enum values name *what the mode does to your data*, legibly, not a modelling-methodology acronym (`docs/research/20260703-model-updates.md` §17.4).

**A smelt-owned pattern, distinct from engine-owned SCD.** `versioned` is one of the patterns smelt maintains itself — it owns the combiner (close-old / open-new) and validates the mode against the derived properties rather than choosing it (`model_maintenance.md` §"Validator, not chooser"). An *engine-maintained* SCD2 is not a variant of this mode — it is hand-written SCD2 SQL declared `refresh: materialized_view`, where the engine's IVM runtime does the maintenance (`materialized_view.md` §Design "No named pattern"). The two are not `versioned` + a maintainer flag; they are different modes with different freshness owners (`docs/research/20260703-model-updates.md` §17.8).

**The combiner stays local; the driver and `merge_into` are referenced.** Close-old / open-new is meaningful only inside this mode, so it lives here in full (`model_transforms.md` §"Transforms that stay in a mode spec"). The mechanisms it is emitted *through* — keyed `merge_into`, the windowed-keyed-maintenance driver, source-filter pushdown — are general capabilities referenced by name, not re-specified.

**Derive from SQL where possible.** Following the keyed-mode posture, the natural key and tracked attributes should be derived from the SQL and the model's declared key rather than restated in a strategy block wherever that is unambiguous (`keyed_models.md` §Design). The precise derive-vs-declare line for change-tracking columns is an Open Question.

## Constraints & Invariants

1. **`refresh: versioned` implies `table` storage.** No `materialization:` restatement.
2. **No `timeseries:` and no `batched:` block on the model itself.** Keyed + interval output; not a partitioned batched build. Window-forward consumption of a `timeseries:` *source* is derived and in-bounds (§"Input consumption").
3. **Validity intervals are non-overlapping per key.** At most one open (`is_current`) version per key at any time; closed intervals abut at shared boundaries with no gaps.
4. **Validity is stamped from source event-time, never the run clock.** This is what makes the mode order-independent and replay-safe.
5. **End-state equivalent and order-independent** (`model_maintenance.md` §"The equivalence invariant"). Merging non-overlapping snapshots in any order converges to the same version history.

## Known Divergences / Open Questions

- **Not implemented — does not parse.** `RefreshStrategy` (`crates/smelt-core/src/config.rs`) accepts only `full` / `batched` / `cumulative` / `materialized_view`; `refresh: versioned` fails deserialization with an `Invalid refresh strategy` error today. The classifier, the close-old / open-new maintenance (via `merge_into`), and the validity-column management are delivered by `docs/plans/20260704-model-updates.md`.
- **Validity-column surface is unsettled.** Exact names/types of `valid_from` / `valid_to` / `is_current`, whether the open interval uses NULL or a sentinel far-future timestamp, and whether these are configurable are Open Questions to settle when the mode is built.
- **Tracked-attribute selection is unsettled.** All projected non-key columns vs an explicitly declared subset; how a modeller marks a column untracked. Prefer deriving from SQL over a strategy block; the exact line is undecided.
- **Late corrections to a closed interval.** Deletion is settled as a soft-close (§"Deletion handling"), but how a correction to an *already-closed* interval is applied — and any opt-in hard-delete surface — need their own design, the same retraction question the keyed modes share (`keyed_models.md` §"Reprocessing"; `docs/research/20260703-model-updates.md` §18.2).
- **Umbrella subsumption.** Whether `versioned` shares execution machinery with the other keyed modes or is a standalone rule is settled here as **standalone** (its own spec, its own classifier), consistent with the narrow-composable-rules posture (`docs/research/20260522-cumulative-as-its-own-rule.md`). It composes shared capabilities by name but owns its combiner.

## References

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy` — `versioned` not yet a variant); on build, the classifier under `crates/smelt-logical/src/rules/` and the maintenance path under `crates/smelt-runtime/`.
- **Related specs**:
  - [`model_maintenance.md`](model_maintenance.md) — the equivalence invariant (interval-keyed variant), the algebraic ladder, the composition contract, validator-not-chooser
  - [`model_properties.md`](model_properties.md) — the monotonicity/ordering discriminants, driving-fact resolution, event-time trace, window-independence / ordered-execution
  - [`model_transforms.md`](model_transforms.md) — keyed `merge_into`, the windowed-keyed-maintenance driver, source-filter pushdown (close-old / open-new stays local here)
  - [`models.md`](models.md) — the refresh axis; the three-state declaration law; the input-consumption axis; `versioned` as a keyed-output peer
  - [`keyed_models.md`](keyed_models.md) — the peer keyed mode covering the overwrite (Type-1), running-aggregate, and milestone patterns; the reference keyed-maintenance path
  - [`materialized_view.md`](materialized_view.md) — engine-owned maintenance (where hand-written SCD2 SQL goes instead)
  - [`timeseries.md`](timeseries.md), [`sources.md`](sources.md) — the world-facts (clock; mutation profile) this mode consumes
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming); Part 19 (the input-consumption axis)
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance framework this mode composes into
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches (`scd2`, `latest_value`, `accumulating_snapshot`)
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
</content>
</invoke>
