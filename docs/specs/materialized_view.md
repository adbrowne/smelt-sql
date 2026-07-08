---
feature: materialized_view
status: experimental
last_reviewed: 2026-07-04
owners: [andrew]
---

# Materialized-View Refresh Mode

> **What this is.** A normative spec for `refresh: materialized_view` — the keyed-output refresh mode whose freshness is owned by the **execution engine**, not by smelt. It is the **delegation target** where the algebraic maintenance ladder ends: smelt runs no combiner and keeps no maintenance state, handing the model's logical SQL to the backend's native incremental-view-maintenance (IVM) runtime (Databricks Enzyme, Snowflake Dynamic Tables, …), which keeps the result current continuously. Covers the composition of the mode, the frontmatter selector, the freshness contract that distinguishes it from `keyed`, the no-silent-fallback rule, and the local machinery unique to delegated maintenance. Out of scope, with their own homes: the equivalence invariant, the algebraic ladder, and the composition contract (`model_maintenance.md`); the backend capability flags (`multi_backend.md`); the delegate-to-native-IVM transform mechanism (`model_transforms.md`); the smelt-owned keyed modes (`keyed_models.md`, `versioned_models.md`); the storage/refresh axes (`models.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. Implementation status lives in §Known Divergences (with plan link) or §References → Plans (history).

## Surface

### Composition

Per the composition contract (`model_maintenance.md` §"The composition contract"), this mode is a composition of the capabilities below. It is the extreme case: it requires **essentially no smelt-side property** — it runs no combiner, does no eligibility analysis of its own, and keeps no maintenance state. Its one input is a backend world-fact.

| Facet | Value |
|---|---|
| Properties required (smelt-side) | **none** — smelt proves nothing about the SQL; it does no native-IVM eligibility analysis of its own (§Semantics → *No smelt-side eligibility*) |
| World-facts consumed | `supports_native_ivm` — the backend capability flag (`multi_backend.md` §"Incremental-view-maintenance capabilities") |
| Transform driven | **delegate-to-native-IVM** (`model_transforms.md`) — emit the backend's own maintained object; hard-error if the engine rejects the query |
| Output shape | **engine-defined** — whatever shape the delegated query produces (no smelt-imposed key or `partition_column`; `models.md` §"Refresh axis") |
| Freshness owner | **the engine (push)** — the engine keeps it current continuously between runs (§Semantics → *Freshness owner*) |
| Equivalence discharged by | the **engine's native IVM**, not the smelt oracle — smelt runs no combiner for this mode (`model_maintenance.md` §"The equivalence invariant") |

This is where **smelt-driven maintenance ends and delegation begins**: rungs 1–4 of the algebraic ladder are what smelt maintains itself; beyond the ladder — general-operator retraction over joins, unbounded non-additive state — is delegated to the engine via this mode (`model_maintenance.md` §"The algebraic maintenance ladder").

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

It **forbids** a `timeseries:` block and a `batched:` block — the output shape is engine-defined, not a smelt-declared partitioned table (`models.md` §"Constraint violations"), and maintenance is the engine's, not a batched DELETE+INSERT. A consumer-facing `timeseries:` declaration on the output — so downstream pushdown could work against a partitioned engine-maintained view — is a plausible future direction but is not admitted today (§Known Divergences).

Any SQL the backend's IVM runtime accepts is eligible — including hand-written SCD, join, or `DISTINCT` logic. There is no smelt-side eligibility analysis; eligibility is exactly what the engine incrementalises (§Design → *No named pattern*).

### CLI

A `refresh: materialized_view` model is created and handed to the engine by `smelt build`; the engine thereafter maintains it continuously without a per-run smelt invocation. It does **not** consume the `--event-time-start`/`--event-time-end` run-window flags — smelt does not own its freshness cadence.

## Semantics

### Freshness owner: the engine (push)

`refresh: materialized_view` and `refresh: incremental` + `grain: key` both uphold **end-state equivalence** (`model_maintenance.md` §"The equivalence invariant") over the inputs processed so far, though `materialized_view`'s output shape is engine-defined rather than smelt's specific one-row-per-key contract (§"Output shape"). They differ in **who discharges the invariant** and in **who owns freshness**:

| | `grain: key` | `materialized_view` |
|---|---|---|
| invariant discharged by | the smelt equivalence oracle (smelt runs the combiner) | the **engine's native IVM** (smelt runs no combiner) |
| freshness model | **pull** — correct as of the last `smelt build` | **push** — engine keeps it current continuously |
| cadence owner | smelt | the engine |
| "is it up to date?" | after the last run | between runs too |

That different operational commitment — a different answer to "is this table fresh, and who is responsible" — is why `materialized_view` is a peer refresh value rather than a hidden maintainer flag on `grain: key`.

### Engine-incrementalizability (the delegated eligibility gate)

Whether a given query can be maintained as a native incremental view is the **engine's** verdict, not smelt's. This is the mode-local eligibility gate: smelt submits the SQL and the engine's IVM runtime **accepts or rejects** it. smelt relays that verdict verbatim (§*No smelt-side eligibility*). A rejection is a hard error carrying the engine's own reason.

### No smelt-side eligibility

smelt performs **no** native-IVM eligibility analysis of its own. It does not attempt to predict, from the SQL, whether the engine will incrementalise the query — it submits the query and relays the engine's accept/reject verbatim. This is deliberate (§Design): re-deriving what each engine's IVM already decides would duplicate engine-specific logic smelt does not own, and would risk smelt rejecting a query the engine would have accepted (or vice versa).

### No silent fallback

Because the refresh modes are peers and smelt never chooses a mode for the user (`model_maintenance.md` §"Validator, not chooser"; `models.md` §Design), `refresh: materialized_view` cannot silently degrade to another mode:

1. **Backend has no native IVM** (e.g. DuckDB; `supports_native_ivm = false`) → **hard error**: *"`refresh: materialized_view` requires native incremental-view maintenance; this engine has none — use `refresh: incremental` with `grain: key` for smelt-driven maintenance."* smelt does **not** quietly substitute `grain: key` (that would swap the declared freshness contract) and does **not** fall back to a full-refresh table. This is the one carve-out from the backend's lower-don't-reject rule (`multi_backend.md` §Semantics).
2. **Backend has native IVM but rejects the query** → **hard error** carrying the engine's own reason (e.g. Enzyme's `MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`). smelt surfaces the backend's message rather than masking it.

The inability to rescue the user into a different mode is the honest price of peer status; it keeps the refresh enum coherent (each value means exactly one thing).

### Output shape

The output shape is **engine-defined**: whatever the delegated SQL and the engine's native maintenance produce, with no smelt-imposed key or partition column. This differs from the smelt-owned keyed modes (`keyed_models.md`, `versioned_models.md`), whose output shape is a specific, smelt-defined contract (one row per `unique_key`, or a validity-interval-keyed table) — here smelt does not shape the output at all, because it neither classifies nor combines the SQL. Downstream models reference it as an ordinary relation; no partition filter is pushed into it. Adding a consumer-facing `timeseries:` declaration on this output (so downstream pushdown could target a partitioned engine-maintained view) is accepted as a future direction, deferred pending pushdown wiring (§Known Divergences).

## Design

**A peer refresh value, not a maintainer flag.** `materialized_view` earns a peer name because it changes the *freshness owner*, an operational contract the user must see — not merely an implementation detail. Modelling it as `cumulative: { native: true }` was rejected: it would surface a physical-execution choice (who maintains the state) as a sub-knob on a logical mode, exactly the metadata-vs-SQL drift the peer-enum design fights (`models.md` §Design; `docs/research/20260703-model-updates.md` §16, §17.5).

**Not a storage mode.** An engine-maintained materialized view persists data (storage = `table`) and is kept current by the engine (a refresh property). Putting it on the storage axis — as an earlier `materialization: materialized_view` value did — repeated dbt's conflation of storage and refresh. It lives on the refresh axis where the freshness-owner question belongs (`models.md` §Design).

**No named pattern (why there is no `versioned + native` cell).** smelt-owned maintenance requires smelt to recognise the combiner, hence the specific patterns `keyed` (its column families) / `versioned`. Engine-owned maintenance requires **no** named pattern: the engine's IVM runtime incrementalises arbitrary eligible SQL. To get an engine-maintained SCD2, the modeller writes the SCD2 logic in SQL and declares `refresh: materialized_view` — there is no `versioned`-native variant to fill, because native maintenance never needed the pattern name. This is why the two axes (pattern, maintainer) do not multiply into a combinatorial enum, and why no `maintained_by:` modifier is required (`docs/research/20260703-model-updates.md` §17.8).

**Delegation inherits the engine's correctness; smelt relays, it does not re-decide.** Unlike the smelt-driven keyed modes, smelt keeps no maintenance state and runs no combiner for this mode; correctness, retraction, and incrementalisation eligibility are the engine's, and the equivalence invariant is discharged by the engine's native IVM rather than the smelt oracle. smelt's job is to emit the native maintained object and surface the engine's accept/reject verdict legibly. Assuming a full-SQL backend and relaying (not re-deriving) the engine's eligibility verdict is deliberate: the smelt-specific value is the surface and the freshness contract, not re-implementing what each engine's IVM already decides.

## Constraints & Invariants

1. **`refresh: materialized_view` implies `table` storage.** The modeller never restates `materialization:`.
2. **No `timeseries:` and no `batched:` block.** Output shape is engine-defined; engine-maintained.
3. **Never silently falls back.** A backend without native IVM, or an IVM runtime that rejects the query, is a hard error — never a silent switch to `keyed` or `full`. This holds at every layer that could emit or substitute the object.
4. **smelt owns no maintenance state and runs no combiner for this mode.** The engine keeps state and presentation; smelt emits the object and relays engine diagnostics.
5. **smelt does no native-IVM eligibility analysis of its own.** It relays the engine's accept/reject verbatim; it never pre-flights or overrides the engine's verdict.

## Known Divergences / Open Questions

- **The compile-time capability gate hard-errors; the backend emit path silently falls back.** The gate is wired and tested at compile time: `refresh: materialized_view` parses (`RefreshStrategy::MaterializedView` in `crates/smelt-core/src/config.rs`), its constraint violations are enforced, and `crates/smelt-runtime/src/compile.rs` hard-errors when `supports_native_ivm = false` (the §"No silent fallback" case 1 message, asserted by `test_materialized_view_hard_errors_without_native_ivm`). Because every backend today sets `supports_native_ivm = false` (`multi_backend.md`), that gate always fires first, so the mode never reaches emit. **But** the backend-level default is inconsistent with Constraint 3: `Backend::create_materialized_view_as` (`crates/smelt-backend/src/lib.rs`) defaults to `create_table_as` **with a warning** — a silent fallback to a plain table — rather than hard-erroring. This is latent today (the compile gate shields it) but must become a hard error before any backend sets `supports_native_ivm = true` without implementing native IVM, else a mis-capable backend would silently emit a stale table. Closing both — the honest emit against a real IVM backend (Databricks Enzyme is the reference target) and removing the backend fallback — is tracked by `docs/plans/20260704-model-updates.md`. Aligned with `model_transforms.md` §Known Divergences, which records the same `create_materialized_view_as` fallback as a *partial* delegate-to-native-IVM transform.
- **No backend advertises native IVM.** DuckDB and both Spark profiles set `supports_native_ivm = false` (`multi_backend.md`), so `refresh: materialized_view` currently always resolves to the §"No silent fallback" hard error. The mode is specified ahead of a backend that provides it.
- **The hard-error message still names `cumulative`, pending the keyed-collapse rename.** §"No silent fallback" case 1 states the smelt-driven alternative as `refresh: keyed`, the mode's timeless name; the shipped message and its test (`test_materialized_view_hard_errors_without_native_ivm`) still say `refresh: cumulative`, because `refresh: keyed` does not parse yet (`keyed_models.md` §Known Divergences). The message text is renamed as part of the keyed-collapse work (`docs/plans/20260705-keyed-collapse.md`).
- **Backend eligibility surfacing is minimal by design.** smelt performs no native-IVM eligibility analysis of its own; it relies on the backend to accept or reject the query and surfaces the engine's reason. Richer smelt-side pre-flight (predicting incrementalisability before submission) is out of scope until a concrete backend motivates it (`docs/research/20260703-model-updates.md` §18.2).
- **Per-engine physical-strategy modifier is deferred.** If a single engine exposes multiple native IVM implementations of the same view (distinct refresh algorithms), smelt would pick a per-engine default and let the user override it — a physical-strategy override scoped *inside* `materialized_view`, engine-specific and defaulted. This is not a logical-mode selector, so it does not reintroduce the strategy footgun. Deferred until an engine actually presents the choice (`docs/research/20260703-model-updates.md` §17.8).

## References

- **Code**: `crates/smelt-core/src/config.rs` (`RefreshStrategy::MaterializedView`, parsing); `crates/smelt-runtime/src/compile.rs` (the `supports_native_ivm` hard-error gate); `crates/smelt-backend/src/lib.rs` (`create_materialized_view_as` / `drop_materialized_view_if_exists` — today defaulting to a table fallback).
- **Tests**: `test_materialized_view_hard_errors_without_native_ivm` (`smelt-runtime/src/compile.rs`); the `materialized_view` storage-value-rejected and default-materialization-rejected tests (`smelt-core/src/config.rs`).
- **Related specs**:
  - [`model_maintenance.md`](model_maintenance.md) — the equivalence invariant (discharged here by the engine), the algebraic ladder (this mode is where smelt-driven maintenance ends and delegation begins), the composition contract, validator-not-chooser
  - [`model_transforms.md`](model_transforms.md) — the delegate-to-native-IVM transform this mode drives, and backend lowering/emulation
  - [`multi_backend.md`](multi_backend.md) — the `supports_native_ivm` / `supports_retraction` capability flags this mode gates on
  - [`models.md`](models.md) — the refresh axis; why `materialized_view` is an engine-owned freshness peer, not a storage mode; the three-state law
  - [`keyed_models.md`](keyed_models.md) — the smelt-owned keyed peer (a specific smelt-defined output shape and different discharger, vs this mode's engine-defined shape)
  - [`versioned_models.md`](versioned_models.md) — the smelt-owned SCD Type-2 pattern (engine-owned SCD is hand-written SQL under this mode instead)
- **Research**:
  - [`docs/research/20260703-model-updates.md`](../research/20260703-model-updates.md) — Parts 13–17 (keyed/stateful refresh modes, emulation vs delegation, the user surface)
- **Plans (history)**:
  - [`docs/plans/20260704-model-updates.md`](../plans/20260704-model-updates.md) — implements the model-updates research
</content>
</invoke>
