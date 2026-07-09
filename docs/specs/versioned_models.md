---
feature: versioned_models
status: experimental
last_reviewed: 2026-07-07
owners: [andrew]
---

# SCD2 Shape Profile

> **What this is.** The shape profile for `refresh: incremental` + `grain: key` + `versioning: interval` (`models.md` §"Refresh axis"): the stored table is keyed state **plus history** — every version of a key is kept, each stamped with a non-overlapping validity interval — kept current by the derived per-cell maintenance plan (`maintenance_plan.md`) rather than a declared strategy. It is deliberately not a fourth grain: row addressing is still by key; the interval is structure within the key (`models.md` §"Refresh axis"). This spec states which shared **properties** (`model_properties.md`) the profile requires, which **transforms** (`model_transforms.md`) its default plan drives (keyed `merge_into` via the windowed-keyed-maintenance driver), and defines in full the machinery that is profile-**local**: the close-old / open-new interval combiner, the smelt-managed validity columns, tracked-attribute selection, deletion handling, and event-time-stamped validity. It does **not** re-specify a shared capability. Out of scope, with their own homes: the equivalence invariant and composition contract (`model_maintenance.md`); the plan matrix, per-cell admission, and the graph layer (`maintenance_plan.md`); the monotonicity/ordering discriminants, driving-fact resolution, and ordered-execution proofs (`model_properties.md`); keyed `merge_into`, the windowed-keyed-maintenance driver, and source-filter pushdown (`model_transforms.md`); the `refresh:`/`grain:`/`versioning:` surface, the three-state declaration law, and the input-consumption axis (`models.md`); the key-grain peer covering the overwrite, running-aggregate, and milestone patterns (`keyed_models.md`); engine-owned maintenance (`materialized_view.md`); the partition-grain shape profile (`batched_models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history). See the Timeless-oracle rule in `CLAUDE.md`.
>
> **Status: experimental (not yet implemented).** The profile is specified ahead of implementation; the frontmatter surface described here (`grain: key` + `versioning: interval`) does not parse today (§Known Divergences). Delivered by a phase of `docs/plans/20260707-maintenance-plan-impl.md`.

## Surface

### Composition

Per the composition contract (`model_maintenance.md` §"The composition contract"), the SCD2 profile is composed as:

| Kind | What the profile composes | Home |
|---|---|---|
| **Output shape / grain** | `grain: key` + `versioning: interval` — the key-grain sub-declaration that keeps every version with a validity interval instead of only the current row | `models.md` §"Refresh axis" |
| **Properties (required)** | algebraic monotonicity / ordering discriminants (value-monotone vs order-monotone, for tracked-attribute change detection); **event-time monotonicity trace** (validity is stamped from source event-time, never the run clock); **driving-fact / anchor resolution** (the single clocked source under window-forward); **window-independence / ordered-execution** (the combiner reads versions in event order) | `model_properties.md` |
| **World-facts (consumed)** | the **timeseries clock** of an update-events / CDC feed (`timeseries.md`), *or* a mutable snapshot's **source mutation profile** (`sources.md`) — one of the two, derived from the source's shape, never declared on the model | `timeseries.md`, `sources.md` |
| **Default plan (fold-a-delta corner)** | keyed **`merge_into`** sequenced by the **windowed-keyed-maintenance driver**, with **source-filter pushdown** on the driving source, folding through the **close-old / open-new interval maintenance** combiner (profile-local, below) | `model_transforms.md` |
| **Admission** | every check below is one instance of `maintenance_plan.md` §"Per-cell admission" evaluated for the fold-a-delta corner over a key-grain-plus-interval output (§"Admission" below) | `maintenance_plan.md` |
| **Invariant upheld** | end-state equivalence in its **interval-keyed specialisation** — the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same processed snapshots, independent of merge order (§Semantics) | `model_maintenance.md` §"The equivalence invariant", `maintenance_plan.md` §"Per-cell admission" |

The normative content of this spec is that table plus the profile's **local** machinery defined below: the close-old / open-new combiner, the smelt-managed validity columns, tracked-attribute selection, event-time stamping, and deletion handling.

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: incremental
grain: key
versioning: interval
unique_key: [customer_id]
---

SELECT
    customer_id,          -- the natural key
    tier,
    region
FROM smelt.customers_snapshot
```

`versioning: interval` is admitted only on `grain: key` (`models.md` §"Constraint violations") and is a hard error together with a `timeseries:` block on the model itself — the SCD2 close-out escapes every time window (`models.md` §"Constraint violations"). This forbids output partitioning, not event-time-aware *consumption*: like the plain key grain, a `versioning: interval` model over a source that carries a `timeseries:` declaration (an update-events / CDC feed) consumes that source window-forward (see §"Input consumption").

The model's SELECT projects the **natural key** and the tracked attribute columns as they are *now*. smelt maintains the version history: each `smelt build` compares incoming rows against the stored current version per key and, where a tracked attribute changed, closes the prior version and opens a new one.

### Output shape

Keyed **plus** a validity interval. The stored table carries the projected columns and the smelt-managed validity columns — a `valid_from` / `valid_to` interval and an `is_current` flag (exact names/types are an Open Question). A key with three successive states yields three rows: two closed intervals and one open (`valid_to` NULL / sentinel, `is_current = true`).

## Semantics

### End-state equivalence (interval-keyed)

The profile upholds the **end-state equivalence invariant** in its interval-keyed specialisation (`model_maintenance.md` §"The equivalence invariant"): the user-visible set of `(key, version, validity interval)` rows equals what a full rebuild would compute from the same set of processed snapshots, independent of the order in which non-overlapping snapshots were merged. smelt owns freshness (pull) — the history is correct as of the last `smelt build`.

Order-independence holds because validity is anchored to the source's event time, not the run clock (see §"Validity stamped from source event-time"): the close-old / open-new combiner reads versions in event order via the **driving-fact / anchor resolution** and **ordered-execution** proofs (`model_properties.md`), so replays and out-of-order windows converge to the same history rather than shifting interval boundaries.

### Input consumption is derived from the source

How new input is discovered is never declared on the model; it is the input-consumption axis (`models.md` §"Input-consumption axis"), derived from the source's shape:

- **Window-forward** — a source carrying a `timeseries:` declaration (an update-events / CDC feed) is consumed in `--event-time` run windows applied to the *source's* `partition_column`, exactly as the plain key grain consumes its driving source (`keyed_models.md` §CLI). Only the new tail is read (source-filter pushdown, `model_transforms.md`). Because the close-old / open-new combiner consumes versions in event order, windows are applied in temporal order (ordered execution, `model_properties.md`).
- **Snapshot-diff** — a mutable snapshot source (no monotone clock) is re-scanned each run and compared against the stored current versions; the end-state contract is identical, only the scan cost differs.

The choice between the two is the mutation-profile world-fact (`sources.md`) feeding the input-delta-discovery proof (`model_properties.md`); moving along this axis never changes the equivalence contract, only what is scanned.

### Admission

Every admission check for this profile is one instance of `maintenance_plan.md` §"Per-cell admission" evaluated for the fold-a-delta corner over a key-grain-plus-interval output:

- **Replayable input / faithful fold** (obligations 1–2) — the close-old / open-new combiner consumes an update-events / CDC feed (replayable, append-only) or a mutable snapshot (re-scanned whole each run); either discharges the obligation for its own consumption route, never a hybrid of the two on one model.
- **Combiner algebra class** (obligation 3) — the combiner is the profile's own local machinery (below), not a catalogued key-grain column family; it is admitted once per model, not per column, because every tracked attribute is folded through the same close-old / open-new step.
- **Bounded reach / bounded footprint** (obligations 4–5) — window-forward: the reach is the run's event-time window on the driving source, exactly as the plain key grain (`keyed_models.md` §"Admission matrix"); the footprint is the set of keys touched by that window's rows. Snapshot-diff: reach and footprint are the whole snapshot and the whole key space — an intentional escape hatch for a source with no monotone clock, not a derivation gap.
- **Well-defined groups** (obligation 6) — all tracked attributes plus the validity columns form one column group; a version change is a single indivisible event across every tracked column, so there is no sub-model factoring to compute.

## Profile-local machinery

The following is owned in full by this spec — it is the machinery meaningful only inside `versioning: interval`.

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

A key present in the store but absent from the incoming set is a **retraction**, and how it is handled is settled here as a soft-close: the key's current version is closed (`valid_to` set, `is_current = false`) with no new version opened, marking "no longer present as of this event time." The event time used is the run's window boundary for a window-forward feed, or the snapshot's as-of time for snapshot-diff. A hard delete (physically removing the key's rows) is **not** the default — the whole point of `versioning: interval` is to retain history — but the exact surface for opting into a hard delete, and for *late corrections* to an already-closed interval, remain Open Questions (they are the retraction question the key grain shares; `keyed_models.md` §"Reprocessing"). A CDC feed that carries explicit delete events resolves this directly: the delete event is the close signal.

## Design

**A sub-declaration of the key grain, not a fourth grain.** `versioning: interval` composes onto `grain: key` rather than introducing a peer grain: row addressing is still by key, and the interval is structure *within* the key, not a different addressing scheme (`models.md` §"Refresh axis"). This is the shape-profile demotion's consequence for the former `refresh: versioned` peer: what changed was never the freshness owner (still smelt-per-run) or the addressing (still by key) — only the local combiner and the extra validity columns, which the litmus rule (`models.md` §Design) says are derived machinery, not grounds for a new enum value.

**A smelt-owned pattern, distinct from engine-owned SCD.** This profile is one of the patterns smelt maintains itself — it owns the combiner (close-old / open-new) and validates the profile against the derived properties rather than choosing it (`model_maintenance.md` §"Validator, not chooser"). An *engine-maintained* SCD2 is not a variant of this profile — it is hand-written SCD2 SQL declared `refresh: materialized_view`, where the engine's IVM runtime does the maintenance (`materialized_view.md` §Design "No named pattern"). The two are not this profile plus a maintainer flag; they are different modes with different freshness owners (`docs/research/20260703-model-updates.md` §17.8).

**The combiner stays local; the driver and `merge_into` are referenced.** Close-old / open-new is meaningful only inside this profile, so it lives here in full (`model_transforms.md` §"Transforms that stay in a mode spec"). The mechanisms it is emitted *through* — keyed `merge_into`, the windowed-keyed-maintenance driver, source-filter pushdown — are general capabilities referenced by name, not re-specified.

**Derive from SQL where possible.** Following the key-grain posture, the natural key and tracked attributes should be derived from the SQL and the model's declared key rather than restated in a strategy block wherever that is unambiguous (`keyed_models.md` §Design). The precise derive-vs-declare line for change-tracking columns is an Open Question.

## Constraints & Invariants

1. **`versioning: interval` is admitted only on `grain: key`.** No `materialized_view` restatement; the opt-in implies `table` storage (inherited from the key grain).
2. **No `timeseries:` block on the model itself, together with `versioning: interval`.** Keyed + interval output; not a partitioned build. Window-forward consumption of a `timeseries:` *source* is derived and in-bounds (§"Input consumption").
3. **Validity intervals are non-overlapping per key.** At most one open (`is_current`) version per key at any time; closed intervals abut at shared boundaries with no gaps.
4. **Validity is stamped from source event-time, never the run clock.** This is what makes the profile order-independent and replay-safe.
5. **End-state equivalent and order-independent** (`model_maintenance.md` §"The equivalence invariant"). Merging non-overlapping snapshots in any order converges to the same version history.

## Known Divergences / Open Questions

- **Not implemented — does not parse.** `RefreshStrategy` (`crates/smelt-core/src/config.rs`) accepts only `full` / `batched` / `cumulative` / `materialized_view`; there is no `grain:`/`versioning:` frontmatter surface at all today (`models.md` §Known Divergences), so `versioning: interval` fails deserialization. The classifier, the close-old / open-new maintenance (via `merge_into`), and the validity-column management are delivered by `docs/plans/20260707-maintenance-plan-impl.md`.
- **Validity-column surface is unsettled.** Exact names/types of `valid_from` / `valid_to` / `is_current`, whether the open interval uses NULL or a sentinel far-future timestamp, and whether these are configurable are Open Questions to settle when the profile is built.
- **Tracked-attribute selection is unsettled.** All projected non-key columns vs an explicitly declared subset; how a modeller marks a column untracked. Prefer deriving from SQL over a strategy block; the exact line is undecided.
- **Late corrections to a closed interval.** Deletion is settled as a soft-close (§"Deletion handling"), but how a correction to an *already-closed* interval is applied — and any opt-in hard-delete surface — need their own design, the same retraction question the key grain shares (`keyed_models.md` §"Reprocessing"; `docs/research/20260703-model-updates.md` §18.2).
- **Umbrella subsumption.** Whether this profile shares execution machinery with the plain key grain or is a standalone classifier is settled here as **standalone** (its own classifier), consistent with the narrow-composable-rules posture (`docs/research/20260522-cumulative-as-its-own-rule.md`). It composes shared capabilities by name but owns its combiner.

## References

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy` — no `grain`/`versioning` surface yet); on build, the classifier under `crates/smelt-logical/src/rules/` and the maintenance path under `crates/smelt-runtime/`.
- **Related specs**:
  - [`model_maintenance.md`](model_maintenance.md) — the equivalence invariant (interval-keyed variant), the algebraic ladder, the composition contract, validator-not-chooser
  - [`maintenance_plan.md`](maintenance_plan.md) — the plan matrix, per-cell admission, and the graph layer this profile's admission instantiates
  - [`model_properties.md`](model_properties.md) — the monotonicity/ordering discriminants, driving-fact resolution, event-time trace, window-independence / ordered-execution
  - [`model_transforms.md`](model_transforms.md) — keyed `merge_into`, the windowed-keyed-maintenance driver, source-filter pushdown (close-old / open-new stays local here)
  - [`models.md`](models.md) — the refresh axis; the three-state declaration law; the input-consumption axis; `versioning: interval` as a key-grain sub-declaration
  - [`keyed_models.md`](keyed_models.md) — the peer key-grain profile covering the overwrite (Type-1), running-aggregate, and milestone patterns; the reference keyed-maintenance path
  - [`materialized_view.md`](materialized_view.md) — engine-owned maintenance (where hand-written SCD2 SQL goes instead)
  - [`timeseries.md`](timeseries.md), [`sources.md`](sources.md) — the world-facts (clock; mutation profile) this profile consumes
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Part 17 (the user surface; naming); Part 19 (the input-consumption axis)
  - [`docs/research/20260704-maintenance-fundamentals.md`](../research/20260704-maintenance-fundamentals.md) — the maintenance framework this profile composes into
  - [`docs/research/20260705-refresh-as-maintenance-plan/`](../research/20260705-refresh-as-maintenance-plan/) — the shape-profile demotion and per-cell admission this profile composes
  - [`docs/research/20260522-cumulative-as-its-own-rule.md`](../research/20260522-cumulative-as-its-own-rule.md) — the sibling-rule sketches (`scd2`, `latest_value`, `accumulating_snapshot`)
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
  - [`docs/plans/20260707-maintenance-plan-impl.md`](../plans/20260707-maintenance-plan-impl.md) — lands the target frontmatter surface (`grain`/`versioning`) and diagnostics
