---
feature: model_transforms
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Model Transforms

> **What this is.** The catalogue of **physical execution mechanisms** a model's
> properties license — general, reusable transforms usable well beyond refresh
> maintenance (backfills, schema evolution, general execution). Each transform
> names the property or world-fact that licenses it, the mechanism it emits, and
> the invariant it preserves. It defines *mechanisms*, not *modes*: it does not
> decide when a mode selects a transform (that composition is `model_maintenance.md`),
> nor prove the properties that license one (`model_properties.md`), nor own the
> **processed-input equivalence invariant** the transforms serve (defined once in
> `model_maintenance.md` §"The equivalence invariant" — referenced here, never
> redefined). Out of scope, with their own homes: mode-only transforms that are
> meaningless outside a single `refresh:` mode (`batched_models.md`,
> `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`,
> `accumulating_snapshot.md`); the backend capability flags a transform's lowering
> checks (`multi_backend.md`); the `refresh:` enum and the three-state declaration
> law (`models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the transforms as if they have always existed. Implementation status lives in §Known Divergences (behaviour + plan link) or §References → Plans (history).

## Surface

The callers of this spec are the refresh-mode specs, the planner, and the
runtime emitter. The surface is the transform catalogue: each transform is a
named physical mechanism, licensed by exactly the property or world-fact in its
row, that a mode composes by name.

**Placement criterion.** A mechanism is catalogued here **iff it is
meaningful/stateable without naming a single refresh mode** — a general
capability a property licenses. A mechanism meaningful only inside one mode
stays in that mode's spec (see §Semantics → *Transforms that stay in a mode spec*).

| Transform | Licensed by (property / world-fact) | Mechanism | Maturity |
|---|---|---|---|
| Keyed `merge_into` (target-as-replica) | inverse-free / monoid rung | fold a keyed delta into stored state; matched keys update, unmatched insert; never re-read history | **built** |
| Windowed-keyed-maintenance driver | driving-fact / anchor + monoid rung | sequence `merge_into` across driving partitions: classify → step over partitions in temporal order → per-partition pushdown → create-or-merge | *partial* (cumulative-orchestration today) |
| Source-filter pushdown (window-an-input) | monotonicity trace + derived bound | wrap each bounded input ref in a `partition_column` subquery so the scan reads only its window | **built** |
| Partition DELETE+INSERT | trace + partition alignment | delete the touched half-open partition range `[start, end)`, then insert the rebuilt rows | **built** |
| Outer output-clamp | event-time projection (needs no proof) | filter the outermost `SELECT` on the projected `event_time` to the write window | **built** |
| Two-layer widened-scan + exact output clamp | finite frame reach `k` | scan `[start − k − offset, end)`, clamp output to `[start, end)`: read the margin, never re-write it | *partial* (redesign) |
| UNION-branch wrap-and-filter | set-operation distribution + per-branch trace | inject the source filter independently into each `UNION`/`INTERSECT`/`EXCEPT` branch | unbuilt |
| Hidden decomposed state + presentation view | decomposed-monoid rung | store the monoid element (`(sum,count)` / Welford / HLL), expose the user value through a pure presentation view `π(state)` | unbuilt |
| Retraction via delta history | group (invertible) rung | store the invertible per-partition delta; on reprocessing subtract the old contribution, then add the new | unbuilt |
| Explicit bounded-domain multiset state | bounded-domain budget assertion | store a per-key value→count multiset (a bounded-domain Z-set); one state serves many presentations and free retraction | unbuilt |
| Compile-time pinning | run-determinism (`NOW`/`CURRENT_*`) | resolve a run-deterministic function to a single literal once per run, before emit | unbuilt |
| Targeted column backfill | additive-only model diff | `UPDATE`/dimension-merge only the added columns in place, never a full rebuild | unbuilt (new) |
| Dimension-driven horizon-bounded MERGE | target-as-replica + join-contribution monotonicity + horizon `H` | merge a dimension batch straight into the target slice `[conv_ts − H, conv_ts]`; never re-read the fact | unbuilt (new) |
| Idempotent window re-scan vs delta-driven probe | idempotent monoid + source mutation profile | unconditional CDF-free re-scan when the fold is idempotent; a per-run changed-set probe when a change feed is available | *partial* |
| Delegate-to-native-IVM | `supports_native_ivm` + engine gate | emit the backend's own maintained object; hard error if the engine rejects the query | *partial* |
| DAG composition | litmus rule (`models.md`) | express a mode combination as two composed models at two grains, not a new mode | mechanism exists |
| Full refresh | — (universal fallback) | drop and rebuild the whole output; the honest verdict for an unmaintainable declared mode | **built** |
| Backend lowering / emulation | backend capability flags | lower a logical primitive to the engine (native `INSERT OVERWRITE` / create-or-replace, or DELETE+INSERT emulation; cross-engine `read_parquet`) | **built** |

## Semantics

Every transform preserves the **processed-input equivalence invariant**
(`model_maintenance.md`): the physical result equals what a full refresh over the
same processed inputs would produce. A transform that **cannot** preserve it for a
given model is **refused with a diagnostic, never applied approximately** (see
§Constraints). The load-bearing mechanics:

**Keyed `merge_into` (target-as-replica).** The stored table *is* the keyed state
(one row per key). A run computes the delta over new inputs and folds it into the
target — matched keys update, unmatched insert — without re-reading history. Sound
only on the monoid rungs of the ladder (`model_maintenance.md`); an invertible
combiner is required to *un-see* a contribution under reprocessing (handled by
retraction-via-delta-history, not by `merge_into` alone). The step loop that
sequences `merge_into` across driving partitions (classify → step → per-partition
pushdown → create-or-merge) is the *windowed-keyed-maintenance driver* — a mode-agnostic
mechanism, so it is catalogued here in its own right. Its reference *implementation*
lives today in `cumulative_aggregate.md` (the only keyed mode built so far) and
generalises to the other keyed modes as they land; the normative *description* of the
driver mechanism is this catalogue entry, not the mode spec.

**Source-filter pushdown + the two clamps.** Three related mechanisms share one
window. Source-filter pushdown wraps each bounded input ref in a subquery so the
scan is pruned at the source. The outer output-clamp filters the outermost
projection so only the write window is emitted. When the model has a **finite
frame reach `k`** (a `RANGE … INTERVAL` window, an interval join), the two-layer
widened-scan reads `[start − k − offset, end)` — wide enough to compute the window
correctly at the left edge — while the output clamp still restricts writes to
`[start, end)`: the margin is *read but never re-written*, which is what keeps the
result partition-equivalent. For the transparent single-source, zero-margin case
the pushdown filter *is* the clamp (same window by construction) and the outer
clamp is dropped as textually redundant. UNION-branch wrap-and-filter is the same
pushdown distributed independently over each set-operation branch.

**Hidden decomposed state + presentation view.** The stored column is a monoid
element that is not itself the user value; the user value is a pure function
`π(state)` exposed through a presentation view. `merge_into` maintains the state
element; `π` never touches history. Sound iff `π` is a pure function of one
consistent state row.

**Retraction via delta history** and **explicit bounded-domain multiset** are the
two ways to regain fidelity the plain monoid loses: the first stores invertible
per-partition deltas so a contribution can be subtracted before re-adding (the
group rung, for corrections/deletes); the second stores a per-key value→count
multiset so exact holistic aggregates (`MEDIAN`, exact `COUNT(DISTINCT)`) are
maintainable. The multiset is **opt-in and fail-loud**: state is `O(active
domain)`, so it is applied only under a declared bounded-domain budget and the
runtime caps it with a full-refresh fallback.

**Targeted column backfill** and **dimension-driven horizon MERGE** avoid a full
rebuild for two enrichment shapes. Backfill is licensed by an *additive-only model
diff* — an edit that only adds columns derivable from `{existing target} ∪
{monotone dimension}` — and edits those columns in place. The horizon MERGE is
licensed by target-as-replica **plus** a monotone join contribution **plus** a
bounded horizon `H`: a dimension batch merges directly into the target slice
`[conv_ts − H, conv_ts]` without re-reading the fact. Both refuse (fall back to
rebuild) the moment their licensing property does not hold.

**Compile-time pinning** resolves a run-deterministic function (`NOW()`,
`CURRENT_DATE`) to one literal per run so every partition of that run sees the same
value — the equivalence invariant would otherwise be violated by a value that
drifts across a chunked backfill. A genuinely non-deterministic function
(`RANDOM`, `UUID`) is *not* pinnable and is refused unless the column is declared
exempt.

**Delegate-to-native-IVM, full refresh, backend lowering.** Delegation emits the
engine's own maintained view and hard-errors when `supports_native_ivm` is false —
it runs no smelt combiner and is where smelt-driven maintenance ends. Full refresh
is the universal fallback that always upholds the invariant trivially. Backend
lowering/emulation maps a logical primitive to whatever the engine supports
(native `INSERT OVERWRITE`, or DELETE+INSERT emulation; cross-engine transfer via
Parquet), gated on the backend capability flags in `multi_backend.md`.

### Transforms that stay in a mode spec

These are meaningful only *inside* one refresh mode and are **not** catalogued
here; the mode spec owns them in full:

- **Backfill chunking** (one-shot / auto-sized / per-partition) and **auto-coarsen
  run window** — `batched_models.md`.
- **Close-old / open-new interval maintenance** (SCD2) — `versioned_models.md`.
- **Upsert-overwrite** (overwrite per key) — `latest_value_models.md`.
- **Eviction / settled-key GC** (retire keys older than `current_window − H`) —
  `accumulating_snapshot.md`.

## Design

**Named for the mechanism, not the mode.** A keyed `merge_into`, a source-filter
pushdown, a targeted backfill are each useful for backfills, schema evolution, and
general execution — not only refresh maintenance — so they are catalogued as
capabilities and composed by name, rather than re-described inside each mode's
spec. This is what gives `smelt:validate` one home per mechanism to check for
drift.

**The placement criterion is definitional, not a consumer count.** A transform
lives here iff its mechanism is stateable without naming a mode. This is why
pushdown, DELETE+INSERT and the clamps live here even though only `batched` drives
them today, while backfill chunking — which has no meaning outside batched
execution — stays in `batched_models.md`. Building a transform before a second
consumer exists is fine (they are broadly useful); the criterion governs *where the
text lives*, never *when it is built*.

**A property licenses; it never chooses.** Each row names exactly the property or
world-fact that makes the transform sound. The transform is applied only when that
licence holds and is otherwise refused — the machinery is a validator, never a
chooser (`model_maintenance.md` §"Validator, not chooser"). This keeps the mapping
property → transform auditable and forbids an approximate application when the
licence is absent.

**The ladder is the maintainable/delegated boundary.** `merge_into`,
decomposed-state-plus-view, retraction, and the multiset are the mechanisms of
rungs 1–4 of the algebraic ladder; delegate-to-native-IVM is what lies beyond it.
The ladder itself (its ordering and cutoff) is owned by `model_maintenance.md`;
this spec only realises each rung as a physical transform.

**Rejected: auto-widening the write window.** An earlier runtime widened the
*written* partition range to cover a window's lookback rather than only widening
the *scan*. That double-counts at partition edges and is being redesigned into the
two-layer widened-scan/exact-clamp split (read the margin, write only the window).
The write window must equal the output window; only the scan may be wider.

## Constraints & Invariants

- **Equivalence or refusal.** A transform is applied only when its licensing
  property is proven or declared. A transform that cannot preserve the
  processed-input equivalence invariant (`model_maintenance.md`) for a given model
  is **refused with a diagnostic** — never applied approximately, and never with a
  silent fallback to a default.
- **Write window = output window; scan window ⊇ output window.** Any widened-scan
  transform may read a margin but must clamp writes to exactly the output window.
- **`merge_into` requires a monoid rung; reprocessing requires invertibility.**
  A non-invertible combiner under reprocessing is refused (or routed to full
  refresh), never merged approximately.
- **The bounded-domain multiset is opt-in and capped.** It is applied only under a
  declared budget and fails loud (full-refresh fallback) when the domain exceeds
  the cap; it is never the default for a holistic aggregate.
- **Native-IVM delegation is fail-loud.** Delegate-to-native-IVM hard-errors when
  the backend does not support it; it never silently downgrades to a smelt-driven
  approximation.
- **One home per mechanism.** A transform meaningful only inside a mode is not
  catalogued here; the mode spec owns it (see §Semantics).

## Known Divergences / Open Questions

The whole catalogue is being consolidated and largely unbuilt; status is tracked
by `docs/plans/20260704-model-updates.md` (design:
`docs/research/20260704-maintenance-fundamentals.md`).

- **Built today:** keyed `merge_into` (the `Backend::merge_into` trait method,
  impls in `smelt-backend-duckdb`/`-spark`); source-filter pushdown
  (`inject_source_filters`); partition DELETE+INSERT (`delete_partitions` +
  `insert_into_from_query`); outer output-clamp (`inject_time_filter`); full
  refresh; backend lowering/emulation (`insert_overwrite`, cross-engine Parquet).
- **Two-layer widened-scan is a *partial* redesign.** The runtime currently
  over-widens the *written* window and under-reads the *scan* window; the
  read-margin/write-window split described in §Semantics is not yet the emitted
  behaviour. The transparent single-source, zero-margin fast path
  (`is_transparent_single_source`) is built.
- **Delegate-to-native-IVM is partial:** `create_materialized_view_as` currently
  falls back to a plain table with a warning on backends without native support,
  rather than hard-erroring per §Constraints.
- **Unbuilt:** UNION-branch wrap-and-filter, decomposed-state-plus-view,
  retraction via delta history, bounded-domain multiset, compile-time pinning,
  targeted column backfill, dimension-driven horizon MERGE. Idempotent re-scan vs
  delta probe is partial (input-delta discovery is partial).
- **Duplicated licensing analyses.** Several transforms are licensed by proofs that
  exist in duplicate in the tree (two interval-reach analyses, two driving-fact
  resolvers, two bound-derivation orchestration sites); the licences these
  transforms read are being consolidated in `model_properties.md`. Until then a
  transform may read the derived bound from more than one code path.
- **Watermark settled-delay / tail-rewrite** (delay writes until a forward-reach
  source has settled, or rewrite the tail) is a shared transform for the forward-
  reach case that is not yet specified here; open, tracked by the same plan.
- The **windowed-keyed-maintenance driver** is catalogued above but only *partially*
  built: the loop that sequences `merge_into` across driving partitions is exercised
  today only by `cumulative`, whose implementation is the reference path. It generalises
  to the other keyed modes as they land; the mechanism's normative home is this spec,
  not the mode that first implements it.

## References

- **Code**: `crates/smelt-backend/src/lib.rs` (`merge_into`, `delete_partitions`, `insert_into_from_query`, `insert_overwrite`, `create_materialized_view_as` trait methods); impls in `crates/smelt-backend-duckdb`, `crates/smelt-backend-spark`; `crates/smelt-runtime/src/transformer.rs` (`inject_source_filters`, `inject_time_filter`, `is_transparent_single_source`); `crates/smelt-runtime/src/compile.rs`.
- **Tests**: the batched per-partition full-refresh-equivalence harness; the cumulative end-state-equivalence harness; the pushdown/clamp unit tests in `smelt-runtime/src/transformer.rs`; the generative soundness oracle.
- **User docs**: the per-mode refresh pages under `docs-site/docs/`.
- **Plans (history)**: `docs/plans/20260704-model-updates.md`.
- **Related specs**: `model_maintenance.md`, `model_properties.md`, `models.md`, `batched_models.md`, `cumulative_aggregate.md`, `latest_value_models.md`, `versioned_models.md`, `accumulating_snapshot.md`, `materialized_view.md`, `multi_backend.md`, `timeseries.md`, `sources.md`, `schema_evolution.md`.
