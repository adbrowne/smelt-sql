# Spark Declarative Pipelines (and Databricks Lakeflow) — state of the art, read against smelt

**Date:** 2026-08-18
**Status:** research, non-normative
**Question:** what does SDP/Lakeflow actually do today in the areas smelt claims as its differentiators — incrementalization, schema evolution, backbuilds, batch/streaming unification, and engineer control — and where does that leave smelt?

---

## 1. TL;DR

SDP is the strongest incumbent in smelt's problem space, and since June 2025 it is no longer proprietary: Databricks donated its DLT engine to Apache Spark as **Spark Declarative Pipelines** ([SPARK-51727](https://issues.apache.org/jira/browse/SPARK-51727)), shipping in Spark 4.1 ([programming guide](https://spark.apache.org/docs/latest/declarative-pipelines-programming-guide.html)). Databricks' hosted version is **Lakeflow Spark Declarative Pipelines** ([docs](https://docs.databricks.com/aws/en/ldp/concepts/)), the rename of Delta Live Tables.

The split matters. **Open-source SDP is a dependency-resolving orchestrator, not an incrementalizer.** It gives you a declarative dataflow graph, streaming vs. materialized dataset types, and a CLI. It has no cost-based incremental maintenance, no CDC operator, and no expectations framework in the published guide. **Everything smelt actually competes with is Databricks-proprietary**: Enzyme (incremental view maintenance), AUTO CDC, expectations, Unity Catalog integration.

The three structural gaps, stated as sharply as I can:

1. **The materialization decision is the user's, not the compiler's.** You write `CREATE STREAMING TABLE` or `CREATE MATERIALIZED VIEW`. That is a *physical* choice embedded in the *logical* definition, and it is irreversible without a full refresh. This is precisely the logical/physical conflation smelt exists to undo.
2. **Where incrementalization *is* automatic (Enzyme), it is a silent, untunable chooser.** It decides per-refresh whether to incrementalize or fully recompute; when it declines, you learn about it *after the run*, from an event-log JSON field. There is no compile-time answer to "will this model be maintained incrementally, and why not?"
3. **Definition changes are not a first-class delta.** Changing an MV's definition triggers a full recompute; several streaming-table schema changes require a full refresh — which itself only works if the source still retains history.

Where SDP is genuinely ahead of smelt: CDC/SCD2 as a built-in operator (smelt explicitly declines this — `incremental_models.md` §"No smelt-maintained SCD2"), streaming sinks, expectations, and a decade of operational surface.

---

## 2. What open-source SDP gives you (Spark 4.1+)

**Programming model.** Three dataset kinds and one flow abstraction:

| Concept | Meaning |
|---|---|
| **Streaming table** (`@dp.table`, `CREATE STREAMING TABLE`) | Append-only incremental processing from a streaming source; exactly-once via checkpoints |
| **Materialized view** (`@dp.materialized_view`, `CREATE MATERIALIZED VIEW`) | Batch-computed table, exactly one batch flow writing to it |
| **Temporary view** (`@dp.temporary_view`) | Pipeline-scoped intermediate; not queryable outside the run |
| **Flow** (`@dp.append_flow`, `CREATE FLOW … INSERT INTO`) | A read → transform → write unit; multiple flows may target one streaming table |
| **Sink** (`dp.create_sink`) | External streaming target (Kafka, Event Hubs). Python-only, append-only |

**Dependencies are implicit**, from `spark.table("x")` or a SQL `FROM` clause. There is no `ref()` equivalent — which means no static distinction between "a model in this pipeline" and "an arbitrary external table", and no ref-level metadata.

**Project shape:** a `spark-pipeline.yml` naming `libraries` (globs of `.py`/`.sql`), `storage` (checkpoint dir), `catalog`/`database`, and a Spark `configuration` map.

**CLI:** `spark-pipelines init | run | dry-run`, built on `spark-submit`. Refresh selection is the interesting part:

```bash
spark-pipelines run                        # incremental update (default)
spark-pipelines run --refresh a,b          # update without clearing
spark-pipelines run --full-refresh a,b     # clear data + checkpoints, recompute
spark-pipelines run --full-refresh-all
spark-pipelines dry-run                    # syntax + analysis + cycle validation
```

`dry-run` is the closest thing SDP has to smelt's compile-time story: it catches syntax errors, unresolved tables/columns, and graph cycles. It does **not** tell you anything about maintainability or plan shape.

**Hard constraints worth noting.** Dataset functions must return a `DataFrame` and must not call `collect()`, `count()`, `toPandas()`, `save()`, `saveAsTable()`, `start()`, or `toTable()` — because SDP *evaluates pipeline code multiple times* across planning and execution, so side effects corrupt the graph. Loop-generated tables require the value list to be "always additive". `PIVOT` is unsupported in SDP SQL. These are the seams of using a general-purpose imperative language (Python) as a pipeline DSL: the framework must forbid by convention what a real compiler would forbid by construction. This is smelt's "proper language instead of Jinja" argument, restated against a different host.

---

## 3. What Databricks adds on top (Lakeflow)

### 3.1 Enzyme — incremental view maintenance

The core proprietary asset ([incremental refresh docs](https://docs.databricks.com/aws/en/optimizations/incremental-refresh)). Named techniques: `ROW_BASED`, `PARTITION_OVERWRITE`, `WINDOW_FUNCTION`, `APPEND_ONLY`, `GROUP_AGGREGATE` — a recognisable analogue of smelt's technique-per-cell plan matrix.

Supported query shapes: SELECT expressions, `GROUP BY`, CTEs, `UNION ALL`, all join types, `WHERE`/`HAVING`/`OVER`/`QUALIFY`, and MVs carrying expectations. Excluded: recursive CTEs and non-deterministic functions (time functions in `WHERE` excepted).

Requirements: **serverless compute only**, **row tracking enabled** on source tables for most techniques, and sources restricted to Delta tables, UC-managed Iceberg, other MVs, or streaming tables.

Control surface — `REFRESH POLICY`:

- `AUTO` (default) — cost model chooses
- `INCREMENTAL` — prefer incremental, silently fall back to full
- `INCREMENTAL STRICT` — **fail rather than fall back**
- `FULL` — always recompute

`INCREMENTAL STRICT` is the one genuinely smelt-shaped lever in the product: it converts a silent cost regression into a loud failure. Note where it sits — a per-view *runtime* policy, not a compile-time proof.

**Diagnosis is post-hoc.** You find out why a refresh recomputed by querying the event log:

```sql
SELECT timestamp, message FROM event_log(TABLE(<table>))
WHERE event_type = 'planning_information'
```

…or `EXPLAIN CREATE MATERIALIZED VIEW`, or the editor's Incrementalization column. The rejection reasons surface as codes like `CHANGESET_SIZE_THRESHOLD_EXCEEDED`, and per a [Databricks Community thread](https://community.databricks.com/t5/data-engineering/ldp-materialized-view-incremental-refreshes-changeset-size/td-p/154744) **the changeset threshold is not user-tunable**. Databricks' own [optimization blog](https://www.databricks.com/blog/optimizing-materialized-views-recomputes) frames the remedy as *refactor your SQL*: remove `CURRENT_DATE()`/`UUID()`, simplify joins, enable row tracking and deletion vectors. Cross joins, full outer joins, semi/anti joins, and "excessive join counts" push you to full recompute.

### 3.2 AUTO CDC

`AUTO CDC` and `AUTO CDC FROM SNAPSHOT` (the rename of `APPLY CHANGES`) compute SCD Type 1 and Type 2 from a change feed or from successive snapshots, handling out-of-order events via a sequencing column, emitting `__START_AT`/`__END_AT` ([docs](https://docs.databricks.com/aws/en/ldp/cdc)). Bitemporal tracking — business time *and* system time — went Beta in May 2026. `FROM SNAPSHOT` is Python-only; sequencing columns must be sortable and non-NULL; requires serverless or Pro/Advanced.

This is a real capability smelt does not have and has deliberately excluded.

### 3.3 Expectations

`CONSTRAINT <name> EXPECT (<cond>) [ON VIOLATION DROP ROW | FAIL UPDATE]`, or `@dp.expect*` in Python ([docs](https://docs.databricks.com/aws/en/ldp/expectations)). Three actions — warn (default), drop, fail — all row-level, with metrics in the pipeline UI and event log. As of January 2026 the *expectation definitions themselves* can live in Unity Catalog tables, version-controlled and shared across pipelines; June 2026 extended `CONSTRAINT` to standalone MVs.

### 3.4 2026 direction

From the [2026 release notes](https://docs.databricks.com/aws/en/release-notes/dlt/2026), the trend is clear — **push declarations out of the pipeline file and into the catalog**:

- Jan: expectations in UC tables; pipeline schedules and configuration read from **UC table properties**; queued execution mode; automatic permission propagation
- Feb: **type widening without pipeline reset**; multi-flow dry-run validation
- Mar: `REPLACE WHERE` flows for incremental batch (Beta); forward references in sink registration; append-once flow dry-run validation
- Apr–Jun: serverless standalone MV/ST (Beta); `REPLACE WHERE` flows gain incremental refresh; bitemporal AUTO CDC; liquid clustering on CDC targets
- Jul: versionless runtime mode; **pipeline checkpoint import** for migration/recovery; catalog commits with DML transactions

`REPLACE WHERE` flows are worth watching — that is region-scoped incremental batch overwrite, i.e. Databricks arriving at something close to smelt's per-cell region write addressing, but as a *user-written* construct rather than a derived one.

---

## 4. The five axes

### 4.1 Engineer control

**SDP/Lakeflow.** The levers are: choose ST vs MV; set `REFRESH POLICY`; pass `--full-refresh`/`--refresh`; refactor SQL to be Enzyme-legible. There is no planner API, no rule mechanism, no way to say "maintain this model *this* way" and have the framework verify correctness is preserved. The optimizer is a black box you negotiate with by rewriting queries and reading logs afterwards.

**smelt.** This is the thesis (`CLAUDE.md` §Key Differentiators #2; `incremental_models.md` §"Validator, not chooser"): the maintenance plan is pure derived data, inspectable before a run, with `maintenance:` overrides that the system *validates* rather than obeys blindly. Compile-time diagnostics answer "why is this model not incrementally maintainable?" — the question Lakeflow answers only in a post-run JSON blob.

**Verdict:** smelt's strongest and most defensible differentiator. `INCREMENTAL STRICT` shows Databricks feels the pain; it is a fuse, not a plan.

### 4.2 Correctness of incremental state

**Lakeflow** offers no stated equivalence guarantee between incremental and full-refresh results. The cost model's freedom to *choose* full recompute is itself the safety net: when unsure, recompute.

**smelt** states the invariant explicitly — `incremental_state(S) == full_refresh(inputs ∈ S)` for every maintained model under any valid run sequence — with a generative conformance gate (`cargo test -p smelt-cli --test maintenance_conformance`) driving typed model recipes through a real backend against a full-refresh oracle. Declared relaxations are confined to a contract lattice (`frozen_horizon`, `deferral`) with a single-owned oracle transform.

**Verdict:** smelt is doing something Databricks does not publish an equivalent of. Whether that is a *marketable* difference or an engineering-internal one is the open question — nobody buys "we have a conformance gate", but they do buy "your dashboard won't silently drift".

### 4.3 Schema evolution

**Lakeflow** ([schema evolution docs](https://docs.databricks.com/aws/en/data-engineering/schema-evolution), [full refresh for STs](https://docs.databricks.com/aws/en/ldp/full-refresh-st)):

- Tolerated without reset: new columns (query auto-restarts), dropped columns as soft deletes (new rows NULL), type widening (since Feb 2026, no reset needed)
- Requires **full refresh**: renames without column mapping, changing dedup columns, type narrowing (`BIGINT`→`INT`), incompatible changes (`STRING`→`INT`), hard column deletes
- Changing an MV's definition at all triggers full recompute

And the sting: *"A full refresh does not reprocess data unless your source retains the full historical dataset."* Schema evolution on a streaming table whose source is a 7-day Kafka retention is not recoverable.

**smelt** treats a definition change as a **definition delta** folded through the same frontier algebra as data deltas, with `smelt migrate` printing a derived migration plan for approval before anything destructive runs (`docs/specs/definition_deltas.md`), plus `smelt diff` classifying column-level changes against the stored schema (`docs/specs/schema_evolution.md`).

**Verdict:** the sharpest capability gap in smelt's favour, and the easiest to demo. "Add a derived column to a 2TB table without rebuilding it, and see the plan first" has no Lakeflow answer.

### 4.4 Backbuilds / backfill

**Lakeflow** has `INSERT INTO ONCE` / `@dp.append_flow(once=True)` ([backfill docs](https://docs.databricks.com/aws/en/ldp/flows-backfill)). The backfill flow stays in the graph as an idle audit record and re-fires on full refresh. Caveats: you must handle duplicates yourself; historical schema must be compatible; AUTO CDC targets reject plain `INSERT INTO ONCE`; SCD seeding requires every key's first live change to sequence after its last seeded change.

This is a good, honest primitive — but it is *loading old data*, hand-written. It is not *deriving how to reconstruct a stored table under a changed definition*, which is what smelt's backbuild option catalogue does from the definition diff, sharing statement emitters with the maintenance layer.

**Verdict:** different problems that share a word. smelt should be careful to say which one it means; "backfill" reads as the Lakeflow meaning to most of the market.

### 4.5 Batch and streaming

**SDP** unifies the *authoring surface* (one file, one graph, one CLI) but forces a **declaration-time** choice between ST and MV, and semantics diverge underneath: time travel works on STs but not MVs; identity columns are recommended for STs only and are recomputed on MV updates; MVs are explicitly discouraged over Kafka/Auto Loader sources ("data sources where records should only process once"); sinks are streaming-only and Python-only. The unification is real at the DAG level and leaky at the semantics level.

**smelt** doesn't offer streaming at all today. This is a genuine coverage gap, not a difference of philosophy — though smelt's derive-don't-declare stance means the ST/MV choice would fall out of the SQL rather than being typed by the author.

**Verdict:** Lakeflow ahead. The interesting smelt position is not "we do streaming too" but "the batch/stream boundary is a planner decision, not a syntax decision."

---

## 5. Testability and portability

- **Local testing:** OSS `spark-pipelines dry-run` catches syntax/analysis/cycle errors. Unit testing in the Lakeflow editor is Beta; the credible local story is a third-party pytest plugin ([godatadriven/sdp-test](https://github.com/godatadriven/sdp-test)) that shims the decorators and strips DDL preambles to run models against a local `SparkSession`. That such a plugin had to exist is the tell.
- **No LSP.** No incremental type checking, no goto-definition on refs, no column-level diagnostics before submit. smelt's Salsa + Rowan + type-oracle stack has no counterpart here.
- **Portability:** OSS SDP runs anywhere `spark-submit` does and reads/writes Delta and Iceberg through standard catalogs. But Enzyme, AUTO CDC, expectations, and UC-stored config are Databricks-only, and Enzyme additionally requires *serverless* compute plus row tracking on sources. The valuable half is not portable, and within Databricks it is coupled to a specific compute tier and table feature set. smelt's multi-backend, logical-first position is the opposite bet.

---

## 6. Reading for smelt

**Sharpen (defensible, demonstrable):**
1. **Compile-time maintainability answers.** The demo is a diagnostic that says "`x` cannot be incrementally maintained because `NOW()` appears outside a `WHERE`" *before* you run — versus Lakeflow's `planning_information` post-mortem.
2. **Definition deltas.** No incumbent equivalent. `smelt migrate` plan-and-approve is a five-minute demo with a visceral payoff.
3. **Plan legibility + overrides that are validated.** "The planner is not a black box" lands harder now that Enzyme's untunable changeset threshold is a documented customer complaint.

**Steal / watch:**
- **`INCREMENTAL STRICT` as a posture.** smelt should have an equivalent: fail the run rather than silently full-refresh a model the user expected to be maintained.
- **Expectations.** Row-level warn/drop/fail with metrics is table stakes; smelt's `data_tests.md` should be read against it.
- **Config in the catalog** (UC table properties). smelt's `smelt.yml`/state split should not assume all config lives in files.
- **`REPLACE WHERE` flows.** Region-scoped incremental batch, arriving user-written. smelt derives the same thing — worth a direct comparison writeup.

**Concede:** CDC/SCD2, streaming sinks, and operational maturity. smelt's `incremental_models.md` §Limitations already declines SCD2 explicitly; that's a defensible scope call, but it needs a stated answer to "so how do I do SCD2 in smelt?"

---

## Sources

Primary:
- [Spark Declarative Pipelines Programming Guide (latest)](https://spark.apache.org/docs/latest/declarative-pipelines-programming-guide.html)
- [SPARK-51727 — SPIP: Declarative Pipelines](https://issues.apache.org/jira/browse/SPARK-51727)
- [What are Lakeflow pipelines? (concepts)](https://docs.databricks.com/aws/en/ldp/concepts/)
- [Incremental refresh for materialized views](https://docs.databricks.com/aws/en/optimizations/incremental-refresh)
- [Backfilling historical data with pipelines](https://docs.databricks.com/aws/en/ldp/flows-backfill)
- [Expectations](https://docs.databricks.com/aws/en/ldp/expectations)
- [AUTO CDC](https://docs.databricks.com/aws/en/ldp/cdc)
- [Full refresh for streaming tables](https://docs.databricks.com/aws/en/ldp/full-refresh-st)
- [Schema evolution in Databricks](https://docs.databricks.com/aws/en/data-engineering/schema-evolution)
- [Lakeflow pipeline limitations](https://docs.databricks.com/aws/en/ldp/limitations)
- [Lakeflow SDP release notes 2026](https://docs.databricks.com/aws/en/release-notes/dlt/2026)
- [Optimizing Materialized View Recomputes (Databricks blog)](https://www.databricks.com/blog/optimizing-materialized-views-recomputes)

Secondary:
- [Databricks Community — MV incremental refresh changeset size](https://community.databricks.com/t5/data-engineering/ldp-materialized-view-incremental-refreshes-changeset-size/td-p/154744)
- [godatadriven/sdp-test — pytest plugin for SDP/LDP](https://github.com/godatadriven/sdp-test)
- [waitingforcode — Lakeflow SDP, flows, private tables, configuration](https://www.waitingforcode.com/databricks/lakeflow-spark-declarative-pipelines-flows-private-tables-configuration/read)

smelt-side references: `docs/specs/incremental_models.md`, `docs/specs/definition_deltas.md`, `docs/specs/schema_evolution.md`, `docs/specs/architecture.md`.
