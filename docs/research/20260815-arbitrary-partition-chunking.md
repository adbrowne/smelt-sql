# Arbitrary-column partition chunking

**Date:** 2026-08-15
**Status:** research — direction agreed in conversation; spec work not yet planned
**Question:** the partition grain's `partition_column` is time-only and monotone-only today. Is that too restrictive — should smelt be able to chunk a model's build by *any* column, not just a time axis, to bound query size and enable parallelism?
**Builds on:** `docs/specs/incremental_shapes.md` §"The partition grain (`grain: partition`)" (declaration, execution model, batch safety classification). Adjacent to `docs/research/20260811-delta-signatures-and-definition-deltas.md`, which reframes grain as a derived output-signature label rather than the front door — this doc treats `partition_column` generalization as orthogonal to that reframe and compatible with it either way.

---

## Background: how partition chunking works today

This section is a self-contained primer so the rest of the doc reads without the spec open.

A `grain: partition` model declares a `timeseries:` block naming an `event_time_column` and a `partition_column`, both required to exist, and `partition_column` must be **monotone** — a timestamp or an ever-increasing integer, checked by the event-time monotonicity trace (`model_properties.md`). Declaring `grain: partition` with no `timeseries:` block is a hard error (`TimeseriesRequiredForPartitionGrain`): today, "partitioned" and "has a clock" are the same declaration.

Everything downstream is built on that monotonicity. A run window `[start, end)` derives, via **Form B** skew inversion, an **output window** — the range of `partition_column` values a write might touch — and the run executes one partition-aligned `DELETE` over that range plus one `INSERT`. The **batch safety classification** (`FullyBatchSafe` / `BoundedSafe(n)` / `PerPartitionOnly`) exists to decide how large a contiguous *time* range can be processed in one query versus split into sequential chunks, and that decision is itself driven by lookback/lookforward bounds derived from the model's sources — a purely time-axis concept (adjacency, skew, ordering). Backfill splits the output window into sequential `DELETE`+`INSERT` pairs in **temporal order**; window-independent models may parallelize that split, but the split points themselves are still time boundaries, and a self-referential (window-dependent) model cannot parallelize at all because later windows depend on earlier ones.

The motivating problem is different from all of this: a single-query full build (or a single-query incremental window) of a large model can spill to disk, run out of memory, or simply take a very long time on engines like Spark — not because the data spans a huge time range, but because it's one big `GROUP BY`/join over a wide table. Splitting that query by *any* well-chosen column — region, customer segment, a hash of a high-cardinality key — into N smaller, disjoint-output queries would bound each query's working set and let them run in parallel, independent of whether the model has a time axis at all.

## Key claims

1. **Chunking is a physical-execution concern, not a maintenance-semantics concern, and today's spec conflates them.** The DELETE+INSERT contract, per-partition equivalence, and batch-safety machinery are about *when a stored partition is touched* (a maintenance/correctness question, answered by time-window derivation). Chunking a build into smaller disjoint-output queries is about *how many queries the engine runs and how big each one is* (a physical-execution question). The two happen to be the same question for a time-monotone `partition_column` today only because time is the one axis the current design can chunk by.

2. **`partition_column` should be freestanding, with `timeseries:` as optional enrichment, not a prerequisite.** A model can declare a physical split key with no clock at all — chunking a full refresh by `region` needs no notion of event time. Adding a clock on top (a composed key/time or time-only shape) layers monotonicity, skew, and lookback derivation onto a column that is *also* usable for chunking on its own terms. `TimeseriesRequiredForPartitionGrain` as currently phrased should not survive: requiring a clock to chunk at all is the restriction being reconsidered.

3. **Chunk determination is column-dependent, not one universal rule.** A monotone time column chunks into contiguous ranges (today's mechanism, kept as-is). A non-monotone column chunks one of two ways depending on cardinality: **discrete-value enumeration** for low-cardinality categorical columns (one chunk per distinct value — `region`, `status`), or a **declared hash-bucket count** for high-cardinality columns with no natural small value set (`customer_id` hashed into N buckets). Both are legitimate and the choice is per-column, not a framework-wide decision — this is closer to how physical partitioning/bucketing is chosen in warehouse table design generally than to a single new algorithm.

4. **Chunks over a non-monotone column are inherently order-independent.** There is no "adjacent chunk" concept for a category or a hash bucket the way there is for time — so the entire skew/lookback/`BoundedSafe(n)`/`PerPartitionOnly` apparatus, which exists to reason about *how much of the neighboring range* a query needs to see, does not apply. The only question is whether the model's SQL has a **cross-chunk dependency** (a global aggregate or a window function with no `PARTITION BY` aligned to the split key) — if not, every chunk is independently computable and parallel-safe by construction; if so, chunking that model is refused rather than silently producing wrong results, the same fail-loud posture the existing safety checks use.

5. **Chunking applies uniformly to full refresh and to incremental partition-grain runs; incremental *narrowing* is a separate, harder question left partially open.** Splitting the write is valuable in both cases — a full refresh of a huge table is exactly the spill/timeout scenario, and it needs no incrementality machinery at all to benefit from chunking. Incrementally narrowing *which* chunks a run touches (skip regions with no new data) needs a way to know which chunk values are dirty: derivable from a clock-bounded delta when the model is composed with a driving source elsewhere, but not derivable at all when there's no clock anywhere — in that case every run must consider every chunk, and chunking only helps by bounding and parallelizing that unavoidable full scan, not by reducing its scope.

6. **Key-grain (`merge_into`) parallelism by a hashed key bucket is a related but separate future opportunity, explicitly out of scope here.** The key grain's fold-a-delta quadrant has its own execution shape (`merge_into` sequenced by the windowed-keyed-maintenance driver) and its own correctness concerns (the merge ledger, reprocessing refusal). Bucketing the *merge target* by a hash of `unique_key` to parallelize `merge_into` itself is a natural extension of the same "chunk by a declared bucket count" idea in claim 3, but it interacts with ledger semantics and per-key addressing in ways this doc has not worked through. Recorded here as a named follow-on, not designed.

## 1. What changes in the declared surface

`partition_column` moves from being a field *inside* `timeseries:` to a freestanding declaration usable with or without a clock:

- **No clock, `partition_column` only** — chunked physical execution, no time semantics, no monotonicity requirement. Applies to both `refresh: incremental` (chunked partition-grain DELETE+INSERT, no window derivation, dirty-chunk detection only where a clock exists elsewhere in the DAG) and to a plain full-refresh model (chunked `CREATE TABLE AS SELECT`, N disjoint `INSERT`s instead of one).
- **Clock + `partition_column`, same column** — today's shape, unchanged: monotone, window-derived, skew-aware.
- **Clock + `partition_column`, different column** — a model with a time axis for maintenance windowing that also wants to chunk its *writes* by a non-time column (e.g., a time-windowed run that additionally splits each window's write by region for parallelism). This is the composition case and needs its own admission story — flagged as an open question below rather than resolved here.

The chunk basis (discrete values vs. hash buckets) needs its own piece of declared surface — at minimum a bucket count for the hash case (`partition_by: { column: customer_id, buckets: 16 }`-shaped, exact grammar TBD) — since smelt cannot infer a good bucket count and enumerating a high-cardinality column's distinct values defeats the purpose.

## 2. Execution model

For a chunk set (values or buckets), each chunk becomes an independent statement pair: a filter on `partition_column` (a value or a `hash(col) % N = i` predicate) pushed into the source scan — the actual fix for the spill/timeout case, since it bounds what each query reads, not just what it writes — and a `DELETE`+`INSERT` (incremental) or plain `INSERT` (full refresh) scoped to that chunk's disjoint output slice. Chunk-safety (claim 4) is checked once per model, not per chunk. Parallelism is the default for a chunk-safe model; sequencing is only forced by a genuine cross-chunk dependency, never by an ordering concern the way time chunking is forced sequential for self-referential models — there is no self-referential analogue on a non-monotone axis.

## 3. Open questions

- **Composition**: how does a model declare "windowed by time, also chunked by region" without the grammar becoming two independent partition declarations that silently interact? Needs its own design pass before this ships.
- **Chunk count / sizing for the hash-bucket case**: does smelt pick a default, require a declared count, or derive one from a cardinality estimate? The declared-count answer is simplest and matches "author states an assumption, smelt checks it" elsewhere in the spec, but a bad choice (too few buckets, one still spills) is a foot-gun worth designing against.
- **Dirty-chunk derivation for the composed case** (clock elsewhere, chunk column is different): claim 5 says this is possible but the mechanism (scan the clock-bounded delta, project distinct chunk-column values) isn't verified against the existing source-filter-pushdown machinery — needs a proof sketch before it's spec-ready.
- **Interaction with the delta-signature reframe** (`docs/research/20260811-delta-signatures-and-definition-deltas.md`): that doc proposes `grain` becomes a derived label on an output signature rather than the front door. If that lands first, chunking should be phrased as a property of a shape profile's *execution*, not a new declared-facts axis — worth sequencing rather than designing twice.

## 4. Scope for a first pass

In scope: freestanding `partition_column` (no clock required), discrete-value and hash-bucket chunk determination, chunked execution for both full refresh and incremental partition-grain, the cross-chunk-dependency safety check.

Out of scope, deferred: key-grain `merge_into` bucket parallelism (claim 6); the clock-plus-different-chunk-column composition case (open question above); any chunk-count auto-tuning.

## References

- Spec: `docs/specs/incremental_shapes.md` §"The partition grain (`grain: partition`)", §"Batch safety classification", §"First-run and backfill".
- Adjacent research: `docs/research/20260811-delta-signatures-and-definition-deltas.md`.
