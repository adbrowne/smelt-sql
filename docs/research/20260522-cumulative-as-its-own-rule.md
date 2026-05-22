# Research: Cumulative materialization as its own planner rule

**Date**: 2026-05-22
**Topic**: Why cumulative / stateful-merge materialization should be a sibling planner rule to incremental rather than a `strategy:` knob inside the incremental rule
**Branch**: worktree-web_analytics
**Predecessor**: `docs/research/20260521-incremental-as-planner-rule.md` (which already names this as a "sibling planner rule" in its Open Questions)
**Motivating example**: `examples/web_analytics/silver/device_user_edges{,_cumulative}.sql` — the per-day-table + cumulative-view split that exists purely because cumulative materialization isn't expressible as a single model today

## Summary

Today the spec keeps `Merge` as a variant of `IncrementalStrategy` (`crates/smelt-core/src/config.rs:323`) with the understanding that a model declaring `incremental: { strategy: merge, unique_key: [...] }` would produce a cumulative table via per-partition UPSERT. The `merge_into` backend trait method is implemented and tested for DuckDB, but no model frontmatter actually reaches it.

This doc argues that wiring it up under `incremental:` is the wrong shape. Cumulative materialization should be a separate planner rule with its own frontmatter block, its own classifier, its own equivalence contract, and its own normative spec. The shared infrastructure (function expansion, dependency graph, the `timeseries:` metadata format consumed *from sources*) already lives in core under the planner-rule design direction; both rules consume from core, neither shares with the other.

A second architectural observation, equally important: **a cumulative model is not itself a timeseries.** Its sources are. The output has a `unique_key` (one row per `(device, user)`) and aggregated columns, but no `partition_column` and no `granularity` — it has collapsed all partitions of history into a single per-key row. The cumulative rule discovers the partition-driving shape by reading `timeseries:` *from the source* declared in its FROM clause; the model itself doesn't declare a timeseries block. Downstream consumers see the cumulative table as a lookup, not a timeseries source.

Three architectural moves carry the recommendation:

1. **Drop `IncrementalStrategy::Merge`.** Incremental is DELETE+INSERT only. The variant misleads readers into thinking MERGE is a within-incremental knob; it is not.
2. **Spec a separate rule** (working name `cumulative_aggregate`). Its frontmatter block declares unique keys and per-column aggregators. The rule discovers its driving partition shape from the timeseries source(s) in its FROM clause; the model itself does not carry a `timeseries:` block. Its safety classifier asks different questions than incremental's. Its equivalence contract is structurally different.
3. **Keep `merge_into` as a backend primitive.** Backends don't care which rule called them; the trait method and DuckDB implementation become the cumulative rule's physical strategy with zero changes.

A further claim, weaker but worth surfacing: cumulative isn't one pattern. SCD2, latest-value tables, and accumulating snapshot facts are all "stateful merge with history" but with different unique-key semantics and aggregator stories. The planner-rule design direction explicitly favors narrow, composable rules. Separate sibling rules per pattern (`cumulative_aggregate`, `scd2`, `latest_value`) compose better than one generic MERGE rule with enough knobs to cover all of them.

## Key Files (current state)

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-core/src/config.rs` | `IncrementalStrategy::Merge` variant, `unique_key` on `IncrementalConfig` | L323, L360 |
| `crates/smelt-backend/src/lib.rs` | `merge_into` trait method (physical primitive); `resolve_strategy` routes to it when `unique_key` non-empty and `supports_merge: true` | L251, L283 |
| `crates/smelt-backend-duckdb/src/lib.rs` | DuckDB `merge_into` implementation + unit tests | L482, L637, L678 |
| `docs/specs/incremental_models.md` | Current spec — keeps `Merge` as a strategy variant; notes "MERGE strategy is DuckDB-only-future" in Known Divergences | §Surface "Strategy enum", §Known Divergences |
| `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md` | Gap #5 — the original framing of the cumulative-materialization need | §Gap #5 |
| `docs/research/20260521-incremental-as-planner-rule.md` | Names this as a sibling planner rule in Open Question #4 | §Open Questions |
| `examples/web_analytics/models/silver/device_user_edges.sql` | Per-day incremental table — workaround half of the split | full file |
| `examples/web_analytics/models/silver/device_user_edges_cumulative.sql` | Cumulative view — workaround other half | full file |

## Architecture & Data Flow

### How the rules diverge

Both rules sit downstream of the same core infrastructure:

```
                           ┌─────────────────────────────────┐
                           │   Core smelt                    │
                           │                                 │
   model SQL ──parse──▶    │  • parser, types,               │
                           │    dependency graph             │
                           │  • function expansion           │
                           │  • timeseries: metadata format  │
                           │    (read from sources)          │
                           │  • planner rule API             │
                           └──────────┬──────────────────────┘
                                      │
                  ┌───────────────────┴────────────────────┐
                  │                                        │
                  ▼                                        ▼
   ┌──────────────────────────────┐     ┌─────────────────────────────────┐
   │  Incremental rule            │     │  cumulative_aggregate rule       │
   │                              │     │                                  │
   │  Model declares timeseries:  │     │  Model does NOT declare         │
   │  on itself (this output      │     │  timeseries: — reads it from    │
   │  is a timeseries).           │     │  driving source(s).             │
   │                              │     │                                  │
   │  Contract:                   │     │  Contract:                       │
   │   per-partition equivalence  │     │   cumulative equivalence after   │
   │   with full refresh          │     │   processing source partitions   │
   │                              │     │   in any order                   │
   │                              │     │                                  │
   │  Classifier asks:            │     │  Classifier asks:                │
   │   does output for partition  │     │   are aggregators commutative    │
   │   D depend only on rows in   │     │   and associative? is the        │
   │   [D - before, D + after)?   │     │   unique_key stable? does the    │
   │                              │     │   SELECT shape match the rule    │
   │  Physical strategy:          │     │   (delta per source partition)?  │
   │   DELETE+INSERT per          │     │                                  │
   │   partition                  │     │  Physical strategy:              │
   │                              │     │   UPSERT (backend.merge_into)    │
   └──────────────────────────────┘     └─────────────────────────────────┘
```

The two rules share *nothing* below the core dividing line. They share the planner-rule API, the inlined CST, the dependency graph, and the `timeseries:` metadata format (which both read from sources, and which incremental additionally writes onto its own output). But those aren't shared *between rules*, they're shared *from core to each rule*. Putting both implementations under one rule trait impl doesn't reduce duplication; it just packs two contracts under one frontmatter block.

### Cumulative outputs are not timeseries

The most important framing the prior version of this doc missed: **a cumulative model has no `partition_column` and no `granularity` on its output.** Its sources do; it consumes them.

Look at `silver/device_user_edges` as the motivating example. The cumulative output schema is:

```
(device_id, user_id, event_count, first_seen, last_seen)
```

There is no `event_date` column. There is no per-partition row shape. Each `(device, user)` pair appears exactly once, with counters that span all of history. The whole point of cumulative is to collapse partitions into a per-key row.

This has three consequences:

1. **The model does not declare `timeseries:` on itself.** The frontmatter shape is just the `cumulative_aggregate:` block.
2. **The rule reads the driving partition shape from the source's `timeseries:` declaration.** The SELECT's FROM clause includes one or more timeseries-tagged sources; the rule uses that source's `partition_column` and `granularity` to step the run loop. For each source partition `D`, the rule filters the source to `[D, D + G)`, runs the delta SELECT, merges into the cumulative target.
3. **Downstream consumers treat the cumulative table as a lookup.** It has no partition column, so there is no filter to push down. A downstream model joining to `device_user_edges` reads it in full each run. This is the same behaviour the incremental design already specifies for sources without a `timeseries:` block (`docs/specs/incremental_models.md` §"Lookup tables").

The asymmetry: **incremental produces a timeseries; cumulative consumes one.** They both depend on the `timeseries:` metadata format, but at different ends of the data flow. Modelling them under the same frontmatter block obscures this; modelling them as sibling rules with distinct frontmatter blocks makes it visible.

**Multi-source disambiguation.** A cumulative model could read multiple timeseries sources with different granularities (rare, but possible — a daily-events source and an hourly-signals source). For v1: refuse — require exactly one timeseries-tagged source. For later: support an explicit `driven_by: <source>` field on `cumulative_aggregate:`. Mismatched granularities are still refused; the `driven_by` field is only for picking among same-granularity sources.

**Zero-timeseries-source cumulative.** A cumulative model that reads only non-timeseries sources has nothing to drive its partition loop and gets refused at planning time. The error message points at the missing `timeseries:` on each source and suggests either declaring it or switching the materialization to `view` / `table`.

### The contract divergence

Incremental's load-bearing property (`docs/specs/incremental_models.md` §"Per-partition equivalence"):

```
∀ p ∈ run window:
  incremental_run().where(partition_col = p)
    == full_refresh().where(partition_col = p)
```

Cumulative materialization *structurally cannot promise this* — and not just because of aggregator algebra, but because the cumulative output has no `partition_col` to slice by. Its point is that the row for `(device_id, user_id)` reflects state across all partitions, not just one. The natural cumulative contract is stated in terms of *source* partitions rather than output partitions:

```
After processing source partitions [D₁, …, Dₙ] in any order:
  cumulative_table  ==  full_refresh(source.where(source.partition_col ∈ [D₁, …, Dₙ]))
```

This is the same shape as a CRDT's eventual-consistency promise: end state depends only on the set of processed partitions, not on the order. To uphold it, the rule has to constrain aggregators to be commutative and associative (sum, min, max, set-union, count, bitwise-or all work; last-write-wins, list-append, non-commutative folds do not). That's an entirely different proof obligation from incremental's "this partition's output depends only on this partition's input window."

### Why per-partition equivalence is the wrong frame for cumulative

You could try to extend incremental's contract by saying "include `partition_col` in `unique_key`." Then `(device_id, user_id, partition_col)` is the key, DELETE+INSERT-by-partition works, the output *has* a `partition_column`, and per-partition equivalence holds. But that's just incremental with a different unique_key — it produces the per-partition shape (one row per `(device, user, date)`), not the cumulative shape (one row per `(device, user)`). It's exactly what `silver/device_user_edges.sql` does today. Adding MERGE to it changes nothing semantically; incremental DELETE+INSERT is already correct for that shape.

The interesting case — and the only one where MERGE actually buys something — is when `partition_col` is **not** in `unique_key` and the output has no partition column at all. Partitions collapse into a single row per key, per-partition equivalence is forfeit by construction, and the model's output is no longer a timeseries. This is the cumulative case, and it's a different rule because it upholds a different contract on a different output shape.

### Different classifiers, in detail

The incremental safety classifier (post the May 2026 work in `crates/smelt-planner/src/rules/incremental.rs`) walks the inlined outer body and rejects constructs that break per-partition determinism: outer-body `OVER` whose `PARTITION BY` doesn't subset the partition column's grouping, `HAVING`, `LIMIT`, subqueries, `DISTINCT`, non-deterministic functions. The per-source bound derivation reads `RANGE BETWEEN INTERVAL '…' PRECEDING` and explicit WHERE/JOIN date filters to compute `(before, after)`.

A cumulative classifier would walk the inlined outer body and reject completely different things:

- **Aggregator algebra.** Each non-key projection in the SELECT must be one of an allowlist of known-commutative, known-associative aggregators (or compose to one). `SUM`, `MIN`, `MAX`, `COUNT`, `BIT_AND`, `BIT_OR`, `BIT_XOR`, `BOOL_AND`, `BOOL_OR`, set-union (via `array_agg(distinct ...)` post-merge), `APPROX_COUNT_DISTINCT` over HLL state — yes. `AVG` — no, but trivially rewritable as `SUM/COUNT` and computed at read time, so an optional rewrite. `STRING_AGG`, `LIST_AGG`, `FIRST`, `LAST` — no.
- **Unique-key stability.** `unique_key` columns must be deterministic over the entire input — no `now()`, no `random()`, no source-rowid leakage.
- **Reprocessing semantics.** What happens when partition D is reprocessed? Reversible aggregators (sum, count) admit subtract-then-add (requires keeping per-partition deltas, or a "reprocess" mode that re-reads the partition's prior contribution). Irreversible aggregators (min, max without auxiliary state) force a full rebuild on reprocessing. The classifier surfaces this as a property; the rule's physical strategy reads the property and either supports reprocessing or refuses it.
- **No OVER / window-frame analysis.** Window functions in cumulative SQL don't make sense — the cumulative state is the window. The classifier rejects them outright in the outer body.
- **No lookback derivation.** The source filter is `[D, D+G)` — only the new partition. Bounds analysis is a no-op (or rather, "bounds are trivially `(0, 0)`"). The per-source bound machinery from incremental doesn't run.

None of these checks share code with the incremental classifier beyond the CST-walking framework, which is core.

### Different SQL shapes

Incremental SQL is *a full SELECT for one partition*. The author writes the query as if it ran in full-refresh mode; the rule filters sources and the outer body to a window.

Cumulative SQL is *a per-partition delta SELECT* that gets merged into the cumulative target. The author writes the query knowing it produces one delta per `(unique_key)` from one source partition's input — for the `device_user_edges` case:

```sql
---
materialization: table

cumulative_aggregate:
  enabled: true
  unique_key: [device_id, user_id]
  aggregators:
    event_count: sum
    first_seen: min
    last_seen: max
---
SELECT
    device_id,
    user_id,
    COUNT(*)        AS event_count,    -- delta: this partition's count
    MIN(event_ts)   AS first_seen,     -- delta: min within this partition
    MAX(event_ts)   AS last_seen       -- delta: max within this partition
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

No `timeseries:` block on the model itself — the output has no partition column. The rule reads `events_parsed`'s own `timeseries:` declaration to know the partition shape to step over.

The rule's job at execution time:

1. Walk the FROM clause, find timeseries-tagged sources, pick the driver (exactly one, in v1 — see "Multi-source disambiguation" above).
2. Source-filter pushdown injects `event_date IN [D, D+G)` onto `events_parsed`, using the driver source's `partition_column` and `granularity`.
3. Engine produces the delta rows (one per key from this partition's events).
4. Rule emits a backend `merge_into` call with `unique_key = [device_id, user_id]` and the aggregator map. Matched rows: combine target columns with delta columns via the declared aggregators (`event_count = target.event_count + delta.event_count`, `first_seen = LEAST(target.first_seen, delta.first_seen)`, `last_seen = GREATEST(target.last_seen, delta.last_seen)`). Unmatched: insert.

The SELECT's columns are deltas, not final values. The aggregator declaration is the merge rule. This is a fundamentally different author surface from incremental, where the SELECT produces final-shape rows that are written verbatim.

An alternative shape — let the SELECT produce *cumulative* rows (the whole history aggregated up to and including `D`) and have the rule recognise it can be incrementally maintained — is much harder analysis and not v1. The delta-SELECT shape is what dbt's incremental MERGE strategy uses too; the prior art is well-trodden.

### Different orchestration semantics

`--auto`'s "what's stale" analysis differs:

| Situation | Incremental | Cumulative (reversible) | Cumulative (irreversible) |
|---|---|---|---|
| New partition D appears | Process partition D | Process partition D, merge in | Process partition D, merge in |
| Existing partition D changes | Re-process partition D (DELETE+INSERT idempotent) | Subtract prior delta + add new delta (if delta history kept), else full rebuild from D | Full rebuild from D forward, or refuse |
| Partition D is deleted | DELETE partition D from output | Subtract its delta if delta history kept, else full rebuild | Full rebuild, or refuse |

The same `--auto` orchestrator can drive all three — it just reads `rule.staleness_response(model, partition)` and acts on the answer. The rule supplies the answer; the orchestrator doesn't care which rule produced the model. This is the planner-rule abstraction working as intended.

### Where MERGE-as-physical-primitive sits

`backend.merge_into(schema, table, source_sql, unique_key)` is a *physical* primitive — "given a query that produces some rows, upsert them by `unique_key`." It says nothing about contracts, classifiers, or rule semantics. The DuckDB implementation already exists and is tested.

The cumulative rule's responsibility on top of the primitive:

- Translate the rule's aggregator declarations into the `source_sql` that `merge_into` receives. Concretely: wrap the user's delta SELECT in a CTE, then write a `SELECT` that, for each matched key, projects `target.col <combine> delta.col` per the aggregator map. Or pass the aggregator map down to the backend and let the dialect-specific MERGE codegen handle it.
- Ensure the `source_sql` is filtered correctly by the source-filter pushdown (which is core machinery — same logic the incremental rule uses, with bounds derivation skipped).
- Handle reprocessing per the classifier's verdict (refuse, or subtract-add with delta history).

`merge_into` itself doesn't move. The trait method stays. The DuckDB impl stays. The unit tests stay. They become the cumulative rule's strategy without any code change.

The corollary: **`IncrementalStrategy::Merge` becomes a dangling variant** if the cumulative rule lives elsewhere. The enum should drop it. The cleanup: delete the variant, delete `resolve_strategy`'s `Merge` branch, delete the dispatcher branch in `execute_model_incremental` (`crates/smelt-backend/src/lib.rs:216`). `merge_into` keeps its trait signature and its tests. The cumulative rule, when it lands, calls `merge_into` directly from its physical strategy.

## Surface sketch (provisional)

```yaml
---
materialization: table

cumulative_aggregate:
  enabled: true
  unique_key: [device_id, user_id]
  aggregators:
    event_count: sum
    first_seen: min
    last_seen: max
    # Future: aggregators with options
    # daily_set: { aggregate: array_union, dedup: true }
    # latest_status: { aggregate: argmax, by: event_ts }
  # Optional explicit driver if multiple same-granularity timeseries sources:
  # driven_by: smelt.silver.events_parsed
---
SELECT
    device_id,
    user_id,
    COUNT(*) AS event_count,
    MIN(event_ts) AS first_seen,
    MAX(event_ts) AS last_seen
FROM smelt.silver.events_parsed
WHERE user_id IS NOT NULL
GROUP BY device_id, user_id
```

No `timeseries:` block on the model — the output has no partition column. The rule reads the driving partition shape from the timeseries source(s) in the FROM clause.

`incremental:` and `cumulative_aggregate:` are mutually exclusive (validation diagnostic `ConflictingMaterializationRules` or similar). `incremental:` requires `timeseries:` on the model; `cumulative_aggregate:` forbids it.

The aggregator allowlist for v1: `sum`, `min`, `max`, `count`, `bool_and`, `bool_or`, `bit_and`, `bit_or`, `bit_xor`. Each maps to a known SQL function and is provably commutative/associative. Extensions (`array_union`, `argmax`, HLL `approx_count_distinct` state) gate behind v2 — they need either richer surface or backend-specific state representations.

## Sibling rules beyond cumulative_aggregate

The same dispatching structure could naturally host other stateful-merge patterns. All of them share the "model output is not itself a timeseries, but the rule consumes a timeseries source" shape. Listing them not to commit to building them, but to test the rule-boundary instinct:

- **`scd2:` (slowly-changing dimensions type 2).** Surface declares `unique_key`, `change_columns`, `effective_from_col`, `effective_to_col`. Output rows have validity intervals but the *table* has no partition column — the same `(unique_key)` value appears in multiple rows with non-overlapping intervals. Classifier reads no aggregators (each dimension's history is per-row, not per-aggregator). Physical strategy: detect changes in `change_columns` for each key, close out the prior row (`effective_to = new event_ts - epsilon`), insert the new row. Equivalence contract: cumulative-after-processing source partitions, like cumulative_aggregate, but the "merge" is row-versioning rather than aggregation.
- **`latest_value:` (currently-true table).** Surface declares `unique_key`, `version_col`. Output has one row per key — the latest one. No partition column on the output. Classifier reads the version column and verifies it's monotonic per key. Physical strategy: upsert when `delta.version_col > target.version_col`. Equivalence contract: end state equals full-refresh, where full-refresh is `ROW_NUMBER() OVER (PARTITION BY unique_key ORDER BY version_col DESC) = 1`.
- **`accumulating_snapshot:` (lifecycle fact tables).** A fact table with milestone timestamps (`order_placed_at`, `order_paid_at`, `order_shipped_at`, …) where each row's columns get filled in as the entity progresses. One row per `unique_key`; no partition column on the output. Surface declares `unique_key`, `milestone_columns`. Classifier verifies each milestone column is once-write (NULL → non-NULL transitions only). Physical strategy: COALESCE-style upsert per milestone column.

Each of these is narrow. Each upholds a different contract. Each produces an output that is *not a timeseries* — consistent with cumulative_aggregate, and the structural reason they all live in sibling rules rather than as variants of incremental. Folding them all into a generic `merge:` rule with enough knobs to cover all of them produces the dbt incremental-strategies kitchen-sink, where the surface ambiguity ("what does `incremental: { strategy: 'merge', incremental_strategy: 'merge', ...}` actually mean?") is the dominant complexity.

The right shape is probably: ship `cumulative_aggregate` first (motivated by the web_analytics example), let the other patterns prove themselves with real demand before specifying them. The rule-API surface stays stable across rules; new rules are additive.

## The case for keeping it under incremental (steelman)

Three arguments for *not* separating, and where each falls down:

1. **"Both consume timeseries + function expansion — separate rules duplicate that read."** They don't. Both rules call into the same core. The cost of "reading timeseries metadata twice" is one query call on each side. There is no implementation duplication beyond what the planner-rule API already shapes.

2. **"Authors think of incremental and cumulative as variants of the same idea."** True at the marketing layer, false at the semantic layer. dbt conflates them under `materialized='incremental'` and dispatches by `incremental_strategy`; this is one of the most common sources of confusion in dbt's documentation and on Stack Overflow. The `strategy:` knob silently changes the equivalence contract — same frontmatter, different invariants. smelt's design pillar (logical/physical separation, explicit contracts) argues for the opposite shape: name the contract in the frontmatter block.

3. **"`merge_into` is already there, just wire it up — minimal work."** Wiring it up under `incremental:` is two days of work. Specifying the cumulative classifier, aggregator allowlist, and equivalence contract is two weeks of work whether it lives in `incremental:` or in `cumulative_aggregate:`. The location doesn't change the work, only the surface clarity.

## Migration impact on `examples/web_analytics/`

With `cumulative_aggregate:` available:

- `silver/device_user_edges_cumulative.sql` is deleted.
- `silver/device_user_edges.sql` shrinks. The frontmatter:
  - Drops the `timeseries:` block (the cumulative output has no partition column).
  - Drops the `incremental:` block; replaced by `cumulative_aggregate: { enabled: true, unique_key: [device_id, user_id], aggregators: {...} }`.
  - The SELECT stays the same (it already produces per-source-partition deltas via GROUP BY).
  - The per-day columns `daily_event_count` / `daily_first_seen` / `daily_last_seen` rename to `event_count` / `first_seen` / `last_seen` — no `daily_` prefix needed because the aggregator does the cross-partition combine.
- Downstream `gold/identity_backward_fill` and `gold/identity_connected_components` switch their FROM from `silver.device_user_edges_cumulative` to `silver.device_user_edges`. Their column references don't change. Their own lookback derivation reads `events_parsed`'s `timeseries:` directly (they already had to, since cumulative-edges-as-view was always a lookup); `device_user_edges` continues to be read as a lookup from their perspective. No behavioural change downstream.
- The README's "two-model split because of smelt limitation" caveat is deleted. Gap #5 in the gap catalogue is closed.

Net: -1 file, ~30 lines removed, one conceptual nuisance gone from the example.

The as-of-day-D divergence (gap #7) doesn't change. `backward_fill` and `connected_components` are still global identity algorithms; their per-partition runs still produce as-of-D outputs that diverge from a full rebuild's final state. `cumulative_aggregate` on `device_user_edges` makes the edges themselves cumulative-equivalent (since sum/min/max are reversible and order-independent), but the downstream algorithms' divergence is a property of the algorithms, not of the edge table.

## Open Questions

1. **Rule name.** `cumulative_aggregate:` is descriptive but long. Alternatives: `aggregate:`, `cumulative:`, `merge:` (overloaded with dbt vocabulary; avoid), `accumulate:`. Want a name that signals "stateful, merges across partitions" and doesn't conflict with future siblings (`scd2`, `latest_value`, etc.). `cumulative_aggregate` is the working name in this doc; not committed.

2. **Multi-source disambiguation.** What if a cumulative model reads multiple timeseries sources with different `partition_column` / `granularity`? Three policies for v1: (i) refuse if there's more than one timeseries source (simplest, surfaces the design choice to the author); (ii) accept multiple same-granularity sources and require `driven_by:` to pick the iteration source; (iii) accept different granularities and run the loop at the finest, widening source filters appropriately on the coarser sources. (i) is the v1 ship-bar; (ii) is the natural follow-on; (iii) is speculative and probably never needed in practice.

3. **What if the FROM clause references the cumulative target itself?** A pattern like "cumulative_state += sum(new_partition) - decay" reads its own prior value. This is recursive: the rule needs to know which sources are "input" (drive the iteration) vs. which is "state" (read the prior cumulative value). Probably refuse in v1 — surface this as a known divergence. Worth thinking through because some real cumulative algorithms (exponential moving average, decaying counters) need it.

4. **Aggregator surface.** Flat string (`event_count: sum`) vs. structured (`event_count: { aggregate: sum }`). Flat is friendlier for v1; structured allows future options (`{ aggregate: array_union, dedup: true }`) without breaking changes. The flat form should probably parse as sugar for `{ aggregate: <name> }` so the surface stays consistent as it grows.

5. **Where does AVG live?** `AVG` isn't commutative-associative on its own. Three options: (i) refuse it, force authors to write `SUM(x) / COUNT(x)` and read it as two cumulative columns; (ii) accept it, store `(sum, count)` as a struct, compute `avg` at read time as a derived column; (iii) accept it with a transparent rewrite (the rule rewrites `AVG(x)` to `SUM(x) / COUNT(x)` at planning time and adds two hidden columns). Option (i) is the v1 default for simplicity. Options (ii) and (iii) are user-experience improvements worth re-visiting later.

6. **Reprocessing semantics.** If partition D's source data changes after D has already been merged in, what happens? Three policies:
   - **Refuse.** Cumulative tables are append-only-merge; the rule errors when asked to reprocess a past partition. Author rebuilds from scratch.
   - **Subtract-then-add (requires delta history).** The rule keeps a side table of per-partition deltas. Reprocessing D subtracts the old delta and adds the new. Only works for reversible aggregators (sum, count, bit_xor); refuses for irreversible (min, max).
   - **Cascade-rebuild.** Reprocessing partition D rebuilds the cumulative state from D onward by re-processing every partition ≥ D in sequence. Always works but expensive.

   v1 likely ships with "refuse" (simplest, surfaces the cost honestly). Subtract-then-add is the most useful upgrade. Cascade-rebuild is the fallback for irreversible-aggregator cases.

7. **Composition with `--auto`.** `--auto`'s "process gaps since last run" needs to know which partitions are stale. For incremental this is per-partition. For cumulative it's "any partition ≥ the earliest stale partition" if any aggregator is irreversible, or "exactly the stale partitions" if all are reversible. The rule supplies this answer; the orchestrator consumes it. The rule-API surface needs a `staleness_response(model, changed_partitions) -> partitions_to_run` hook.

8. **External sources.** A cumulative model reading an external source that itself doesn't have `timeseries:` declared — what's the behaviour? Refuse, per the "zero-timeseries-source cumulative" rule in §"Cumulative outputs are not timeseries". The error message should explain why and suggest declaring `timeseries:` on the source.

9. **Schema evolution.** Adding a new non-key column to a cumulative table is tricky — what's the "cumulative state" for the new column over the historical partitions? Two options: backfill from a `default:` value declared with the aggregator; or refuse to add columns and require a rebuild. Same problem dbt has with incremental MERGE; the prior art there is "on_schema_change" knobs. Out of scope for v1.

10. **Interaction with the projection catalog (incremental Open Question 1).** The deferred projection catalog (`DATE_TRUNC`, `AT TIME ZONE`, etc.) is incremental-specific — it's about reading bounded projections on the source partition column. Cumulative doesn't need it (bounds are trivially `(0, 0)`). The two rules don't share this machinery either.

11. **Should `cumulative_aggregate` ship before the planner-rule refactor lands?** The refactor (moving incremental from in-core to a rule) is large. `cumulative_aggregate` could ship under the current architecture (parallel to incremental, both in-core) and migrate to the rule architecture when that lands. Or it could wait for the refactor. The first option lets the example simplify sooner; the second avoids two migrations. The choice depends on how soon the refactor is scheduled and how loudly the web_analytics example's two-model split bothers users.

## References

- **Predecessor:** `docs/research/2026-05-20-incremental-gaps-from-web-analytics.md` — Gap #5 is this work's motivation.
- **Predecessor:** `docs/research/20260521-incremental-as-planner-rule.md` — names this as a sibling planner rule in Open Question #4.
- **Existing spec:** `docs/specs/incremental_models.md` — what `IncrementalStrategy::Merge` should be removed from.
- **Existing spec:** `docs/specs/timeseries.md` — the shared core metadata both rules consume.
- **Patterns referenced:**
  - dbt's `materialized='incremental' + incremental_strategy='merge'` — the conflated surface this design explicitly rejects. The Stack Overflow / dbt-slack history of "which strategy do I want?" is the evidence that one knob for two contracts is the wrong shape.
  - CRDT design literature on commutative-associative aggregators — the formal basis for the v1 aggregator allowlist and the order-independence equivalence contract.
  - Kimball's dimensional-modelling patterns (SCD2, accumulating snapshot) — the prior art for the sibling rules sketched in §"Sibling rules beyond cumulative_aggregate".
