---
feature: materialized_view
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Materialized-View Refresh Mode

> **What this is.** A normative spec for `refresh: materialized_view` — a keyed-output refresh mode whose freshness is owned by the **execution engine**, not by smelt. smelt hands the model's logical SQL to the backend's native incremental-view-maintenance (IVM) runtime (Databricks Enzyme, Snowflake Dynamic Tables, …), which keeps the result current continuously. Covers the frontmatter selector, the freshness contract that distinguishes it from `cumulative`, the no-silent-fallback rule, and the capability it gates on. Out of scope: the smelt-owned keyed modes (`cumulative_aggregate.md`, `versioned_models.md`, `latest_value_models.md`); the capability matrix itself (`multi_backend.md`); the storage axis (`models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).
>
> **Status: experimental (not yet implemented).** No backend today advertises native IVM, so `refresh: materialized_view` currently always resolves to the hard error below. The mode is specified ahead of a backend that provides it; delivered by a phase of `docs/plans/20260704-model-updates.md`.

## Surface

### YAML frontmatter (in `.sql` files)

```sql
---
refresh: materialized_view
---

SELECT
    customer_id,
    COUNT(*)      AS order_count,
    SUM(amount)   AS lifetime_value
FROM smelt.orders
GROUP BY customer_id
```

`refresh: materialized_view` is the entire opt-in; it implies a stored `table` (the modeller never writes `materialization:` for it — the engine-maintained object is a *refresh* concern, not a storage kind; `models.md` §Design). No other frontmatter key is read or required.

It **forbids** a `timeseries:` block and a `batched:` block — the output is a keyed lookup with no partition column (`models.md` §"Constraint violations"), and maintenance is the engine's, not a batched DELETE+INSERT.

Any SQL the backend's IVM runtime accepts is eligible — including hand-written SCD, join, or `DISTINCT` logic. There is no smelt-side eligibility analysis; eligibility is exactly what the engine incrementalises (Design §"No named pattern").

### CLI

A `refresh: materialized_view` model is created and handed to the engine by `smelt build`; the engine thereafter maintains it continuously without a per-run smelt invocation. It does **not** consume the `--event-time-start`/`--event-time-end` run-window flags — smelt does not own its freshness cadence.

## Semantics

### Freshness owner: the engine (push)

`refresh: materialized_view` and `refresh: cumulative` produce the same-shaped keyed result and both uphold end-state equivalence (`cumulative_aggregate.md` §"Cross-partition equivalence"): the user-visible value equals a full refresh over the inputs processed so far. They differ in **who owns freshness**:

| | `cumulative` | `materialized_view` |
|---|---|---|
| freshness model | **pull** — correct as of the last `smelt build` | **push** — engine keeps it current continuously |
| cadence owner | smelt | the engine |
| "is it up to date?" | after the last run | between runs too |

That different operational commitment — a different answer to "is this table fresh, and who is responsible" — is why `materialized_view` is a peer refresh value rather than a hidden maintainer flag on `cumulative`.

### No silent fallback

Because the refresh modes are peers and smelt never chooses a mode for the user (`models.md` §Design), `refresh: materialized_view` cannot silently degrade to another mode:

1. **Backend has no native IVM** (e.g. DuckDB; `supports_native_ivm = false`) → **hard error**: *"`refresh: materialized_view` requires native incremental-view maintenance; this engine has none — use `refresh: cumulative` for smelt-driven maintenance."* smelt does **not** quietly substitute `cumulative` (that would swap the declared freshness contract) and does **not** fall back to a full-refresh table.
2. **Backend has native IVM but rejects the query** → **hard error** carrying the engine's own reason (e.g. Enzyme's `MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`). smelt surfaces the backend's message rather than masking it.

The inability to rescue the user into a different mode is the honest price of peer status; it keeps the refresh enum coherent (each value means exactly one thing).

### Output shape

The output is a keyed lookup exactly like `cumulative`'s (`cumulative_aggregate.md` §"Output shape"): a unique key, no partition column. Downstream models reference it as an ordinary relation; no partition filter is pushed into it.

## Design

**A peer refresh value, not a maintainer flag.** `materialized_view` earns a peer name because it changes the *freshness owner*, an operational contract the user must see — not merely an implementation detail. Modelling it as `cumulative: { native: true }` was rejected: it would surface a physical-execution choice (who maintains the state) as a sub-knob on a logical mode, exactly the metadata-vs-SQL drift the peer-enum design fights (`models.md` §Design; `docs/research/20260703-model-updates.md` §16, §17.5).

**Not a storage mode.** An engine-maintained materialized view persists data (storage = `table`) and is kept current by the engine (a refresh property). Putting it on the storage axis — as an earlier `materialization: materialized_view` value did — repeated dbt's conflation of storage and refresh. It lives on the refresh axis where the freshness-owner question belongs (`models.md` §Design "materialized_view is a refresh mode").

**No named pattern (why there is no `versioned + native` cell).** smelt-owned maintenance requires smelt to recognise the combiner, hence the specific patterns `cumulative` / `versioned` / `latest_value`. Engine-owned maintenance requires **no** named pattern: the engine's IVM runtime incrementalises arbitrary eligible SQL. To get an engine-maintained SCD2, the modeller writes the SCD2 logic in SQL and declares `refresh: materialized_view` — there is no `versioned`-native variant to fill, because native maintenance never needed the pattern name. This is why the two axes (pattern, maintainer) do not multiply into a combinatorial enum, and why no `maintained_by:` modifier is required (`docs/research/20260703-model-updates.md` §17.8).

**Delegation inherits the engine's correctness.** Unlike the smelt-driven keyed modes, smelt keeps no maintenance state and runs no combiner for this mode; correctness, retraction, and incrementalisation eligibility are the engine's. smelt's job is to emit the native maintained object and surface the engine's errors legibly. Assuming a full-SQL backend and deferring smelt-side eligibility analysis is deliberate: the smelt-specific value is the surface and the freshness contract, not re-deriving what each engine's IVM already decides.

## Constraints & Invariants

1. **`refresh: materialized_view` implies `table` storage.** The modeller never restates `materialization:`.
2. **No `timeseries:` and no `batched:` block.** Keyed output; engine-maintained.
3. **Never silently falls back.** A backend without native IVM, or an IVM runtime that rejects the query, is a hard error — never a silent switch to `cumulative` or `full`.
4. **smelt owns no maintenance state for this mode.** The engine keeps state and presentation; smelt emits the object and relays engine diagnostics.

## Known Divergences / Open Questions

- **The emit path is not implemented; no backend advertises native IVM.** `refresh: materialized_view` parses, its constraint violations are enforced, and the capability gate is wired — but all current backends set `supports_native_ivm = false` (`multi_backend.md`), so the mode always resolves to the §"No silent fallback" hard error today. Emitting the native maintained object against a real IVM backend (Databricks Enzyme is the reference target) is delivered by `docs/plans/20260704-model-updates.md`.
- **Backend eligibility surfacing is minimal by design.** smelt performs no native-IVM eligibility analysis of its own; it relies on the backend to accept or reject the query and surfaces the engine's reason. Richer smelt-side pre-flight (predicting incrementalisability before submission) is out of scope until a concrete backend motivates it (`docs/research/20260703-model-updates.md` §18.2 "Native-IVM delegation scope").
- **Per-engine physical-strategy modifier is deferred.** If a single engine exposes multiple native IVM implementations of the same view (distinct refresh algorithms), smelt would pick a per-engine default and let the user override it — a physical-strategy override scoped *inside* `materialized_view`, engine-specific and defaulted. This is not a logical-mode selector, so it does not reintroduce the strategy footgun. Deferred until an engine actually presents the choice (`docs/research/20260703-model-updates.md` §17.8).

## References

- **Related specs**:
  - [`models.md`](models.md) — the refresh axis; why `materialized_view` is a refresh mode, not a storage mode
  - [`cumulative_aggregate.md`](cumulative_aggregate.md) — the smelt-owned keyed peer (same shape, different freshness owner)
  - [`multi_backend.md`](multi_backend.md) — the `supports_native_ivm` capability flag this mode gates on
  - [`versioned_models.md`](versioned_models.md), [`latest_value_models.md`](latest_value_models.md) — the smelt-owned SCD patterns (engine-owned SCD is hand-written SQL under this mode instead)
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Parts 13–17 (keyed/stateful refresh modes, emulation vs delegation, the user surface)
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
