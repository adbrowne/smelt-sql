# How production systems decide incremental-refresh eligibility

**Purpose:** ground smelt's "expanding incremental eligibility" design (`docs/research/20260701-expanding-incremental-eligibility.md`) in how real systems decide when a SQL model/view can be maintained incrementally vs must be fully recomputed, what they verify vs trust, and what annotations they require. Research date: **July 2026**; per-system version notes inline (these surfaces evolve).

**Reader's map to smelt's design doc.** smelt's rejection catalogue (Part 1): `UNION` at base (E1), subquery-in-`FROM` (B4/E2), joins (Part 5), `DISTINCT` (B6), window functions (B1/C1), non-deterministic functions (B5), `HAVING` (B2), `LIMIT` (B3). smelt's cross-cutting prerequisite: a **monotonicity primitive** — "does this projected `event_time` expression trace back, monotonically, to a real source partition column?" — with three consumers (UNION branches §2.5, subquery/CTE pushdown §4.6, join driving-fact identification §5.4).

---

## Synthesis — the industry consensus and where smelt sits

**1. Everyone draws the same line smelt draws; several publish it as an explicit list.** The construct-level rejection catalogue smelt built from first principles is independently reproduced, near item-for-item, by every *whitelist* engine surveyed — Snowflake Dynamic Tables, BigQuery materialized views, Databricks Enzyme, and (as a changelog "append vs updating" type) Apache Flink. Non-deterministic functions, `UNION` (distinct) vs `UNION ALL`, joins as the hard case, `LIMIT`/Top-N, and window functions recur across all of them. This is strong external validation that smelt's catalogue is *correct*, not merely conservative.

**2. There are two fundamentally different ways to be "incremental," and smelt has picked one.**
   - **Window-forward over a monotone event-time** (smelt's model): read the next time window, assume the source is append-only/monotone so earlier windows are settled. Shared by **cube.dev** (requires a `time_dimension`), **ClickHouse** MVs (physically, append-only insert blocks), **dbt microbatch**, **SQLMesh** `INCREMENTAL_BY_TIME_RANGE`, and — as the streaming form — **Spark Structured Streaming** / **Flink** windowed operators (the watermark *is* the monotone-event-time assertion).
   - **Change-tracking / delta-diffing the source** (no monotone column needed): detect *what rows changed* and propagate the delta. Shared by **Snowflake Dynamic Tables** (Stream-style change tracking), **BigQuery** MVs (storage-metadata append diffing), **Databricks Enzyme** (Delta row-tracking + change data feed), and — the theoretical endpoint — **Feldera/DBSP** (Z-sets: every row carries a ± weight, so inserts/updates/deletes propagate uniformly).

   The trade is explicit: the window-forward camp needs a monotonicity guarantee (smelt's whole primitive) but is simple and needs no per-source change-tracking metadata; the delta camp sidesteps monotonicity entirely but pays with a whitelist + full-refresh fallbacks (Snowflake/BigQuery) or a stateful differential runtime (Feldera). **smelt should state this axis choice explicitly** — its monotonicity primitive is the price of being in the window-forward camp, and DBSP is the standing proof that the whitelist is a *pragmatic* boundary, not a fundamental one.

**3. Nobody else *infers and proves* the driving-fact / monotonicity property — they annotate it. This is where smelt is genuinely novel.** Across every window-forward system, "which column is the event-time clock" and "which input is the driving fact" are supplied by the user, not derived:
   - Spark: `withWatermark(col, delay)` asserts the monotone event-time column; stream-static join *direction* fixes the fact (only the stream can drive); stream-stream outer joins force the user to promote the second input to a clock with a watermark + a time-range predicate.
   - Flink: `WATERMARK FOR ts AS …` in `CREATE TABLE`; temporal join `FOR SYSTEM_TIME AS OF` names the dimension side syntactically.
   - Databricks AUTO CDC: `SEQUENCE BY orderCol` names the ordering clock; `KEYS` names identity.
   - dbt: `event_time=` (microbatch) or a hand-written `is_incremental()` predicate.
   - SQLMesh: `time_column` declared in the model `kind`.
   - cube.dev: a `time`-typed `time_dimension`.

   All of them **trust** that the declared column is genuinely monotone and that per-window evaluation equals whole-table evaluation. **smelt's ambition to *derive* the event-time column from the SQL and *prove* it traces monotonically back to a source partition column is stronger than anything shipped.** The closest existing analog is Flink's internal, optimizer-*inferred* `RelModifiedMonotonicity` (per-column update-increasing/decreasing metadata used to drop retraction state) — but Flink still seeds it from a user-declared watermark. smelt's monotonicity primitive is best understood as "Flink's watermark-attribute, but inferred-and-proven from the projection rather than annotated at the source."

**4. Two design ideas worth importing.**
   - **Eligibility vs cost (from Enzyme).** Databricks doesn't only ask "is this incrementalizable?" — even when it is, a cost model may still choose full recompute (e.g. large source deletes). If smelt ever wants a "partial credit / fall back to full-window recompute rather than hard-reject" mode, Enzyme's pre/post/delta-plan decomposition is the reference. This maps onto smelt's existing `--allow-downgrade` posture.
   - **Non-additive aggregates are a rejection smelt's catalogue doesn't yet name.** Snowflake and BigQuery both explicitly exclude `MEDIAN` / `PERCENTILE_CONT`/`_DISC` / exact `COUNT(DISTINCT)` from incremental refresh (they depend on all rows, not just the window's). smelt's catalogue covers `DISTINCT` but not non-additive aggregates as a class — worth adding as a candidate condition. Note the corollary: `MIN`/`MAX` are *additive-enough* and Snowflake lists them as supported, but they are non-monotone under *deletes* (Flink keeps retraction state for them unless it can prove the column is update-increasing) — smelt's append-only assumption makes them safe where a delta engine must work harder.

**5. The `UNION`-vs-`UNION ALL` split (smelt §2.2) is industry-standard.** Snowflake ("`UNION` behaves like `UNION ALL` + `SELECT DISTINCT`"), BigQuery (`UNION ALL` supported/preview, `UNION DISTINCT` unsupported), and Enzyme (`UNION ALL` supported, plain `UNION` not) all draw exactly smelt's line: the bag-union distributes cleanly, the distinct-union drags in a `DISTINCT` that doesn't. smelt's algebraic argument (§2.2) is the same one these engines encode as a whitelist entry.

---

## Comparison table — smelt's rejection catalogue vs each system

Legend: **✓** incremental-safe / supported · **✗** unsupported → full recompute or rejected · **~** conditional (see notes) · **—** not applicable / not separately documented.

| smelt rejection | Snowflake Dynamic Tables | BigQuery MV (incremental) | Databricks Enzyme (MV) | Spark Structured Streaming | Flink (changelog) | Materialize | dbt / SQLMesh |
|---|---|---|---|---|---|---|---|
| **`UNION ALL`** (E1) | ✓ | ~ (preview) | ✓ (needs row tracking) | ✓ stateless | ✓ (mode = least-monotone branch) | ✓ | trusted |
| **`UNION`/`INTERSECT`/`EXCEPT`** (distinct) | ✓ `UNION` (=UNION ALL+DISTINCT); ✗ `INTERSECT`/`EXCEPT` | ✗ | ✗ plain `UNION` | ✗ (needs dedup) | updating | ✓ | trusted |
| **subquery-in-`FROM`** (B4/E2) | ✓ in `FROM`; ✗ subqueries *outside* `FROM` | ✗ `ARRAY` subqueries | ✗ `SUBQUERY_EXPRESSION_NOT_INCREMENTALIZABLE` (scalar/expr); `FROM`/CTE ✓ | — | ✓ (composed dynamic tables) | ✓ | trusted |
| **joins** (Part 5) | ~ OUTER only w/ equality preds | ~ `INNER` ✓; `LEFT` preview; non-leftmost-table change → full invalidate; ✗ self/full/right outer | ~ inner/L/R/full-outer ✓ (row tracking); ✗ cross/semi/anti/"many joins" | ~ stream-static: stream=fact, only inner/left-outer; stream-stream: outer needs watermark on nullable side + time-range | regular join = updating; interval/temporal join = append | ✓ (materializes both; under-constrained = blow-up) | trusted |
| **`DISTINCT`** (B6) | ✓ | ✓ in SELECT (no exact `COUNT(DISTINCT)`) | ✗ plain `DISTINCT` | ✗ (use `dropDuplicatesWithinWatermark`) | dedup = updating | ✓ (per-group state) | trusted |
| **window functions** (B1/C1) | ✓ mostly (✗ `PERCENT_RANK`, sliding `RANK`, `ANY_VALUE`) | ✗ all analytic funcs | ~ `OVER` ✓ **only with `PARTITION BY`** (`WINDOW_WITHOUT_PARTITION_BY` else) | — (event-time `window()` agg instead) | `OVER` = append (time-bounded); Top-N/`ORDER BY LIMIT` = updating | ✓ (top-k reduce) | trusted |
| **non-deterministic funcs** (B5) | ✗ `RANDOM`/`UUID`/`CURRENT_TIMESTAMP`-in-SELECT (✓ in `WHERE`) | ✗ `RAND`/`CURRENT_DATE`/`SESSION_USER` | ✗ `EXPRESSION_NOT_DETERMINISTIC`; fix = push value into source column | ✗ (breaks reproducible state) | ✗ (breaks deterministic changelog) | ✗ (breaks IVM determinism) | trusted (dbt warns) |
| **`HAVING`** (B2) | ✓ (deterministic) | ✗ (✓ non-incremental only) | ✓ (row tracking) | — | via updating agg | ✓ | trusted |
| **`LIMIT`** (B3) | ✗ `LIMIT`/`FETCH`/`TOP` | — | ✗ (not in supported list) | ✗ | Top-N = updating | ✓ (top-k) | trusted |
| **non-additive agg** *(not yet in smelt's list)* | ✗ `MEDIAN`/`PERCENTILE_*` (✓ `MIN`/`MAX`/`SUM`) | ✗ exact `COUNT(DISTINCT)` (✓ `SUM`/`COUNT`/`MIN`/`MAX`) | — | non-windowed group-by = updating | non-monotone agg = retraction state | ✓ (memory cost) | trusted |
| **change detection** | change-tracking (Streams) + `RELY` keys — **no event_time** | storage-metadata append diff — **no event_time** | Delta row-tracking + CDF — **no event_time** | **watermark** on event_time (annotated) | **watermark** on event_time (annotated) | Z-set weights; `mz_now()` temporal filter | user-annotated cursor / `time_column` |
| **driving-fact / clock** | n/a (diffs source) | leftmost table only | n/a (per-source deltas) | **annotated** (watermark) + join direction | **annotated** (watermark / `FOR SYSTEM_TIME AS OF`) | not annotated (materializes both) | **annotated** (`event_time`/`time_column`) |
| **verifies ≡ full?** | ✓ fails `CREATE` if non-incrementalizable | ✓ (whitelist enforced) | ✓ (algebraic delta or full recompute) | ✓ (engine-maintained) | ✓ (engine-maintained) | ✓ (engine-maintained) | **✗ trusts user** |

---

## Per-system detail

### 1. Databricks — Lakeflow Declarative Pipelines (DLT) + Enzyme

**Streaming Tables (ST) vs Materialized Views (MV).** An ST reads each input row exactly once (append-only incremental ingestion, read via `STREAM(...)`; a change/delete to an existing source row throws — "safest to read from static or append-only sources"). An MV is a precomputed result kept up to date by **Enzyme**, incrementally when possible else full recompute. This split *is* smelt's core question: an ST trusts monotonicity by construction; an MV must prove incrementalizability from the query shape.

**AUTO CDC / `APPLY CHANGES INTO`.** Applies a CDC changefeed to a target ST. Key annotations: **`KEYS`** (row identity), **`SEQUENCE BY orderCol`** (the ordering/clock column resolving out-of-order events — the CDC analog of a watermark's event-time column), optional `STORED AS SCD TYPE 1|2|BITEMPORAL`. **Expectations** (`@dp.expect`, `expect_or_drop`, `expect_or_fail`) are data-quality row predicates and *preserve* incremental refresh.

**Enzyme** (cost-based IVM engine; SIGMOD-Companion paper). Decomposes the query into operators (Project/Filter/Aggregate/Window/Join), builds bottom-up pre-plan/post-plan/**delta-plan** fragments, and chooses `MERGE INTO` vs `REPLACE WHERE` (with an "effectivization" step cancelling redundant insert/delete pairs). Decision is **cost-based, not just eligibility-based**: even when incrementalizable, a cost model (executor-CPU estimate over joins/aggregates/windows/shuffles/scans/writes from historical profiles) may pick full recompute (e.g. large source deletes).

**Published unsupported list** — error class [`MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`](https://docs.databricks.com/gcp/en/error-messages/materialized-view-not-incrementalizable-error-class): `AGGREGATE_NOT_TOP_NODE` (GROUP BY must be top-level), `EXPRESSION_NOT_DETERMINISTIC`, `UDF_NOT_DETERMINISTIC`, `WINDOW_WITHOUT_PARTITION_BY`, `OPERATOR_NOT_INCREMENTALIZABLE`, `SUBQUERY_EXPRESSION_NOT_INCREMENTALIZABLE`, `INPUT_NOT_IN_DELTA`, `ROW_TRACKING_NOT_ENABLED`. Also forcing full recompute: `UUID()`/`RANDOM()`/`CURRENT_TIMESTAMP` (except in `WHERE` time filters — recommended fix is to "push that value into the source table itself," structurally the same as smelt tracing event_time to a source column); cross/full-outer/semi/anti and "large numbers of joins"; `WITH RECURSIVE`. **Supported:** `SELECT` (deterministic), `GROUP BY`+agg, CTEs, `UNION ALL`, `WHERE`/`HAVING`, all 4 join types, `OVER` (with `PARTITION BY`), `QUALIFY`, `EXPECTATIONS`. Base sources limited to Delta/MV/ST/UC-Iceberg; serverless only; many ops need Delta **row tracking**.

**Monotonicity:** MVs do *not* require a driving-fact annotation — Enzyme uses per-source row-tracking + change data feed to detect each source's changeset and propagates deltas algebraically, handling deletes/updates (pricing them, falling back to full recompute). STs require monotonicity, asserted by construction.

Sources: [Incremental refresh docs](https://docs.databricks.com/aws/en/optimizations/incremental-refresh) · [`MATERIALIZED_VIEW_NOT_INCREMENTALIZABLE`](https://docs.databricks.com/gcp/en/error-messages/materialized-view-not-incrementalizable-error-class) · [Optimizing MV recomputes (2025-08-11)](https://www.databricks.com/blog/optimizing-materialized-views-recomputes) · [Enzyme SIGMOD paper notes](https://www.waitingforcode.com/databricks/enzyme-materialized-views-databricks-better-understanding-sigmod-companion-paper/read) · [AUTO CDC INTO](https://docs.databricks.com/aws/en/ldp/developer/ldp-sql-ref-apply-changes-into) · [Lakeflow concepts](https://docs.databricks.com/aws/en/ldp/concepts/). Rules current as of mid-2025 (serverless-only, row-tracking, Iceberg v2/v3 are recent).

### 2. Databricks / Spark Structured Streaming — watermarks

The annotation-based monotonicity model; maps directly onto smelt's monotonicity primitive and driving-fact question.

**Watermark = the annotation.** `withWatermark(eventTimeCol, delay)` declares which column is event time and the allowed lateness. The watermark is a monotone lower bound on completed event-time, tracked as `max(event_time) − delay`. This is exactly smelt's "projected event_time traces monotonically back to a source" — but Spark *asserts* it by annotation and trusts that `max(event_time)` advances. Usable for state cleanup only in **Append/Update** output mode, keyed on the event-time column, with `withWatermark` on the same column before the aggregation. Multi-stream: one **global watermark** at the slowest stream (`spark.sql.streaming.multipleWatermarkPolicy = min|max`).

**Stateful vs stateless.** Stateless (map/filter/project, stream-static joins): trivially incremental. Stateful (aggregations, dedup, stream-stream joins): keep event-time-keyed state; the watermark is what licenses eviction and once-only emission. Without monotone event-time there is no safe eviction point — the mechanistic reason append-only/monotone sources matter.

**Join constraints (the driving-fact answer via asymmetry + annotation).** Stream-static: not stateful, no watermark; **inner and left-outer only** (static on right) — the static side is the lookup/dim, the stream is the driving fact; right/full outer *unsupported*. Stream-stream matrix (Spark 4.x): inner supported (watermark+time-range optional, for cleanup); left-outer **must** watermark the right + time-range; right-outer **must** watermark the left; full-outer **must** watermark one side + time-range; left-semi **must** watermark the right. Rationale: "the engine must know when an input row is not going to match anything in the future." An outer join thus requires promoting the nullable side to a declared monotone clock.

**Explicitly unsupported on streaming Datasets:** `LIMIT`/take-N, `DISTINCT` (use `dropDuplicatesWithinWatermark`), sorting (except post-aggregation in Complete mode), some outer joins, chained stateful ops in Update/Complete, `count()`/`foreach()`/`show()`.

Sources: [Structured Streaming Guide (Spark 4.x)](https://spark.apache.org/docs/latest/streaming/apis-on-dataframes-and-datasets.html) · [Watermarking deep dive (2022-08-22)](https://www.databricks.com/blog/feature-deep-dive-watermarking-apache-spark-structured-streaming) · [Stream-Stream Joins (2018-03-13)](https://www.databricks.com/blog/2018/03/13/introducing-stream-stream-joins-in-apache-spark-2-3.html).

### 3. Apache Flink — dynamic tables & changelog monotonicity

**Model.** A stream is a continuously-changing **dynamic table**; a continuous query produces another dynamic table re-encoded as a **changelog stream**. Correctness contract = smelt's "incremental ≡ full refresh": the running result equals a batch query over all input-so-far. Event time carried by **watermarks**.

**The central classification — append-only vs updating** (a direct analog of smelt's catalogue). Every stream/table is either **append-only** (`+I` only; monotone) or **updating** (retract stream with `-U`/`+U`/`-D`, or upsert stream keyed by a primary key). Flink tracks **ChangelogMode + upsert key** for *every* node and validates operator compatibility bottom-up; an updating branch taints everything downstream unless collapsed. Some operators/sinks *require* append-only input and reject an updating one — this is the enforcement mechanism, expressed as a ChangelogMode type error (smelt's "reject at base").

**Operator classification:** append → windowed aggregation, `OVER`, interval join (time-bounded, watermarks both sides), temporal/versioned-table join (`FOR SYSTEM_TIME AS OF`, dimension lookup), `MATCH_RECOGNIZE`. Updating → non-windowed group-by (retract), regular stream-stream join, `ORDER BY … LIMIT`/Top-N/rank, deduplication, CDC/Debezium source. Windowed aggregation and interval/temporal joins **require a watermark**.

**Monotonicity as an internal notion.** Flink's planner carries **`RelModifiedMonotonicity`** (per-column update-increasing/-decreasing/constant), used e.g. to rewrite `MAX`-with-retract → plain `MAX` when the input is update-increasing (dropping retraction state). This is the closest existing engine analog to smelt's proposed monotonicity primitive — a *derived, inferred* per-column property proving a cheaper execution is sound — but Flink seeds it from the source's declared watermark/PK/changelog-mode. Driving-fact = syntactic (`FOR SYSTEM_TIME AS OF` names the versioned/dim side).

Sources: [Flink SQL Changelog processing (Confluent)](https://developer.confluent.io/courses/flink-sql/changelog-processing/) · [Changelog out-of-orderness (Ververica)](https://www.ververica.com/blog/flink-sql-secrets-mastering-the-art-of-changelog-event-out-of-orderness) · [Dynamic Tables (Confluent)](https://docs.confluent.io/cloud/current/flink/concepts/dynamic-tables.html) · [`FlinkRelMdModifiedMonotonicity` (FLINK-34702)](https://www.mail-archive.com/commits@flink.apache.org/msg60815.html).

### 4. Materialize — differential-dataflow IVM

**Model.** Maintains *arbitrary* SQL incrementally on timely+differential dataflow; every value is a `(data, time, diff)` triple where `diff` is a signed multiplicity. Retractions are first-class, so Materialize supports far more SQL than Flink's append-only path — the cost of generality is governed by monotonicity (memory, not rejection).

**Monotonic vs non-monotonic via `ENVELOPE`.** `ENVELOPE NONE` (default) = append-only ("all records as inserts"), monotonic, stateless — the class where non-monotonic operators use cheap plans. `ENVELOPE UPSERT`/`DEBEZIUM` = non-monotonic, must store current value per key to synthesize retractions (memory ∝ key cardinality). Monotonicity enters at the *source envelope* and changes downstream *plans*, not eligibility.

**Temporal filters and `mz_now()`.** No windowing syntax — windows are ordinary `WHERE` predicates over the logical clock `mz_now()` (e.g. `WHERE mz_now() <= event_ts + INTERVAL '5min'`). As `mz_now()` advances, out-of-window rows are automatically retracted — the temporal filter *is* the row-expiry/TTL and the sliding-window mechanism; temporal-filter pushdown skips old data at storage. Materialization restrictions on `mz_now()`: top-level `WHERE`/`HAVING` conditions must be `AND`-combined only; `mz_now()` compared only to a non-`mz_now()` numeric/timestamp expr; no arithmetic directly on `mz_now()`; can only be materialized if the use is a temporal filter.

**What can be materialized.** Essentially any relational SQL (joins, group-by, distinct, subqueries, unions) — caveats are memory/monotonicity, not outright rejection: non-monotonic `min`/`max` retain state; cross/under-constrained joins blow up state; unbounded views without a temporal filter retain full history.

Sources: [Temporal Filters](https://materialize.com/blog/temporal-filters/) · [Temporal filters pattern](https://materialize.com/docs/transform-data/patterns/temporal-filters/) · [`now`/`mz_now`](https://materialize.com/docs/sql/functions/now_and_mz_now/) · [CREATE SOURCE Kafka (envelopes)](https://materialize.com/docs/sql/create-source/kafka/) · [Self-Correcting Materialized Views](https://materialize.com/blog/self-correcting-materialized-views/).

### 5. dbt incremental models

**Classic pattern.** `materialized='incremental'` + `is_incremental()` guarding a user-written high-water-mark predicate (`where event_time >= (select max(event_time) from {{ this }})`). dbt only requires the SQL to be valid in *both* branches; it does not parse the predicate, does not know `event_time` is a time column, and does not check the predicate captures every row a full refresh would.

**Strategies:** `append` (insert-only, no dedup), `merge` (needs `unique_key`), `delete+insert` (needs `unique_key`), `insert_overwrite` (needs `partition_by`), `microbatch`. `incremental_predicates` narrow the merge/delete scan but are **not validated**.

**Microbatch** reframes incrementality as event-time batching. Mandatory config: `event_time`, `begin`, `batch_size` (`hour`/`day`/`month`/`year`); optional `lookback` (reprocess N prior batches for late data, default 1), `concurrent_batches`. dbt emits one query per batch with an auto-generated half-open `event_time` filter and **auto-filters upstream `ref()`s that themselves declare `event_time`** (an upstream without `event_time` gets a full scan per batch). Framing: "given the same input data, the resulting table is the same no matter how many times a batch is reprocessed" — powering `dbt retry` and backfills. All times assumed UTC.

**Verifies vs trusts — the sharp contrast.** Verifies: SQL valid in both branches, required annotation *present*, `on_schema_change`. **Trusts (never proven):** that the predicate selects every full-refresh row; that `unique_key` is unique (docs warn duplicates "may fail" / nulls "generate duplicate rows"); that transforms are deterministic/idempotent ("non-deterministic or non-idempotent models will produce incorrect results when batches are reprocessed"); logic drift requires a manual `--full-refresh`. Event-time identification is **pure annotation**.

Sources: [Incremental overview](https://docs.getdbt.com/docs/build/incremental-models-overview) · [Strategy](https://docs.getdbt.com/docs/build/incremental-strategy) · [Microbatch](https://docs.getdbt.com/docs/build/incremental-microbatch). (dbt 1.9+ / microbatch GA.)

### 6. SQLMesh

**Model kinds** make incrementality explicit: `INCREMENTAL_BY_TIME_RANGE` (needs `time_column`), `INCREMENTAL_BY_UNIQUE_KEY` (needs `unique_key`; non-idempotent, no partial restatement), `INCREMENTAL_BY_PARTITION` (needs `partitioned_by`), `SCD_TYPE_2`, `FULL`, `VIEW`.

**`INCREMENTAL_BY_TIME_RANGE`.** Two ownership points that differ from dbt: (1) the `time_column` is **explicitly declared** in the `kind`; (2) the user still writes the `WHERE` using `@start_ds`/`@end_ds` macros that SQLMesh substitutes with concrete interval bounds. Separately, SQLMesh **auto-appends a time-range filter to the output query** to guard against writing outside the assigned interval (protects against clobbering on late data). The `time_column` is added to `partitioned_by` by default and drives which intervals restatement overwrites. Materialization is engine-specific (Spark `INSERT OVERWRITE` by partition; Snowflake `DELETE` by range then `INSERT`).

**Stance vs dbt.** Markets stronger correctness — virtual environments (dev/prod share tables via views; `plan` diffs models and computes exactly which intervals to backfill), column-level lineage (real SQL parsing via SQLGlot), a `WHERE FALSE LIMIT 0` probe to validate schema, and an insistence on idempotent incrementals so restatement is safe. **But on the core question it is the same as dbt: it trusts, it does not prove.** Verifies: SQL parses/schema resolves, required annotation present. Trusts: that the `WHERE` actually filters on `time_column` for the interval (docs don't indicate it parses the `WHERE` to confirm); that `unique_key` has no duplicates ("SQLMesh does not automatically detect or prevent duplicates"); that the model is idempotent (recommendation, enforced only structurally). Event-time identification is **pure annotation**, though SQLMesh does more with it than dbt.

Sources: [Model kinds](https://sqlmesh.readthedocs.io/en/stable/concepts/models/model_kinds/) · [Incremental by time guide](https://sqlmesh.readthedocs.io/en/stable/guides/incremental_time/) · [Plans/restatement](https://sqlmesh.readthedocs.io/en/stable/concepts/plans/).

### 7a. Snowflake Dynamic Tables

**Refresh modes.** `REFRESH_MODE = AUTO | INCREMENTAL | FULL`. AUTO resolves once at creation by examining the definition and never re-evaluates. Explicit `INCREMENTAL` on an unsupported definition **fails `CREATE`** (fail-loud, like smelt's diagnostics).

**Supported (near-verbatim):** CTEs (not `RECURSIVE`); `DISTINCT`; deterministic `SELECT`; `WHERE`/`HAVING`/`QUALIFY`; `GROUP BY` (not `ROLLUP`/`CUBE`/`GROUPING SETS`); `INNER`/`CROSS JOIN`; `LEFT`/`RIGHT`/`FULL OUTER JOIN` **with equality predicates only**; `LATERAL FLATTEN` (not its `SEQ`); `UNION ALL`; `UNION` ("behaves like `UNION ALL` + `SELECT DISTINCT`"); window functions **except** `PERCENT_RANK`, sliding-frame `RANK`/`DENSE_RANK`, `ANY_VALUE`; additive/scalar aggregates incl. `SUM`/`COUNT`/`AVG`/`MIN`/`MAX`; `CURRENT_TIMESTAMP`/`_DATE`/`_TIME` only inside `WHERE`/`HAVING`/`QUALIFY`.

**Unsupported → full / error:** `INTERSECT`, `EXCEPT`/`MINUS`; `CONNECT BY`, `RECURSIVE` CTEs, `LIMIT`/`FETCH`/`TOP`, `UNPIVOT`, `SAMPLE`; subqueries **outside** `FROM` (`WHERE EXISTS`, correlated in `WHERE`), sequences; `GROUP BY ROLLUP`/`CUBE`/`GROUPING SETS`; non-deterministic in SELECT (`RANDOM()`, `UUID_STRING()`, `CURRENT_TIMESTAMP`-in-SELECT); session funcs (`CURRENT_USER`/`_ROLE`/`_WAREHOUSE`); **exact/non-additive aggregates** (`PERCENTILE_CONT`/`_DISC`, `MEDIAN`, `APPROX_PERCENTILE`, `APPROX_TOP_K`); `VOLATILE`/external UDFs. Rationale: these "depend on the relationship between all rows, not just the new ones." Note `MIN`/`MAX` are **supported** (unlike BigQuery).

**Annotations:** `TARGET_LAG` (freshness target or `DOWNSTREAM`) + `WAREHOUSE`. **Change detection = change-tracking (Streams metadata) + optional `RELY` PKs — no `event_time`/watermark required.** It diffs the source.

Sources: [Supported queries](https://docs.snowflake.com/en/user-guide/dynamic-tables/supported-queries) · [Refresh modes](https://docs.snowflake.com/en/user-guide/dynamic-tables/refresh-modes) · [Understanding refresh](https://docs.snowflake.com/en/user-guide/dynamic-tables-refresh).

### 7b. BigQuery Materialized Views

Incremental by default; restricted aggregations + SQL syntax (a published incrementalizability whitelist).

**Supported aggregations (final outputs only, verbatim):** `ANY_VALUE`, `APPROX_COUNT_DISTINCT`, `ARRAY_AGG`, `AVG`, `BIT_AND`/`_OR`/`_XOR`, `COUNT`, `COUNTIF`, `HLL_COUNT.INIT`, `LOGICAL_AND`/`_OR`, `MAX`, `MIN`, `MAX_BY`, `MIN_BY`, `SUM`. Aggregate must *be* the output (`COUNT(*)/10` disallowed). No exact `COUNT(DISTINCT)` (use approx / `GROUP BY`).

**Unsupported (incremental):** `UNION ALL` (preview), `LEFT OUTER JOIN` (preview); `UNION DISTINCT`/`INTERSECT`/`EXCEPT`, `RIGHT`/`FULL OUTER JOIN`, **self-joins**, **all analytic/window functions**, `ARRAY` subqueries, **non-deterministic funcs** (`RAND()`, `CURRENT_DATE/TIME()`, `SESSION_USER()`), **UDFs**, `TABLESAMPLE`, `FOR SYSTEM_TIME AS OF`, GenAI funcs. No MV-over-MV; no external/wildcard/view/snapshot sources.

**What forces full refresh:** joins refresh incrementally only when the **leftmost table is appended** — "changes to other tables fully invalidate the view cache." Any **deletion** in a base table → full refresh (for BigLake, always). **Non-incremental MVs** (`allow_non_incremental_definition=true`, GA Apr 2024) allow "most SQL including OUTER JOIN, UNION, HAVING, analytic functions" but "must be refreshed in their entirety" and get no smart-tuning rewrite. Options: `enable_refresh`, `refresh_interval_minutes` (default 30), `max_staleness` (required for non-incremental). **Change detection = storage-metadata append diff — no user column.**

Sources: [Create materialized views](https://cloud.google.com/bigquery/docs/materialized-views-create) · [Intro](https://cloud.google.com/bigquery/docs/materialized-views-intro).

### 7c. ClickHouse Materialized Views

**Insert-trigger model.** A CH MV is "just a trigger that runs a query on blocks of data as they're inserted." On each inserted **block** the MV's `SELECT` runs against *that block only* (never the full source), result inserted into a target table (an aggregating engine merges partials). Unit of increment = the inserted block; no diff, no watermark, no re-scan.

**Pitfalls / limitations:** inserts only — "any changes to existing data of source table (update, delete, drop partition) does not change the materialized view" → silent drift, manual reconciliation. Correctness holds **only if the source is append-only** (the monotonicity assumption, bound to physical insert blocks). **JOINs trigger on the left/source table only** — right-side changes never picked up. Aggregations need `SummingMergeTree`/`AggregatingMergeTree` + `-State`/`-Merge` (can't average averages). `POPULATE` backfill has a documented race (not recommended; unsupported on Replicated/Cloud). **Change detection = append-only INSERT trigger, exactly once per block.**

Source: [Incremental materialized view](https://clickhouse.com/docs/materialized-view/incremental-materialized-view).

### 7d. Feldera / DBSP

**Whole-language IVM, no whitelist.** Claims full-SQL incrementality: relational algebra, joins, `GROUP BY`/aggregations, **correlated subqueries**, **window functions**, `DISTINCT`, set ops, `UNNEST`, UDFs, **recursive queries**. Documented limits are system-envelope (no durability/transactions by default), not SQL surface.

**Theory (the contrast with whitelists).** Views compile to **DBSP circuits**. Incrementalization is `Q^Δ = D ∘ ↑Q ∘ I` and **distributes over composition**: `(Q₁∘Q₂)^Δ = Q₁^Δ ∘ Q₂^Δ`. So once each *primitive* operator has an efficient incremental form, *any composition* is automatically incrementalizable — correctness by theorem, not allow-list. **Change detection = Z-sets:** every row carries a ± integer weight (insert/delete/update = −old+new), so deletions/retractions propagate natively; cost ∝ change size. **No monotone event_time, no windowing, no ordering assumption.** DBSP is the standing proof that smelt's whitelist is a pragmatic engineering boundary, not a fundamental limit — at the cost of a stateful differential runtime rather than "incremental ≡ full over a window."

Sources: [DBSP paper (VLDB 2023)](https://www.vldb.org/pvldb/vol16/p1601-budiu.pdf) / [arXiv 2203.16684](https://arxiv.org/abs/2203.16684) · [Feldera SQL docs](https://docs.feldera.com/sql/intro/).

### 7e. cube.dev pre-aggregations

**Annotation-driven partition refresh.** Incremental refresh composes annotations on a `rollup`: `partition_granularity` (`hour`…`year`, one table per bucket), `refresh_key.every` (cadence), `refresh_key.sql` (change-detection query), `refresh_key.incremental: true` (refresh only recent partitions), `refresh_key.update_window` (rolling window of eligible partitions). Canonical: `partition_granularity: month` + `refresh_key: { every: 1 day, incremental: true, update_window: 3 months }` → refresh the last 3 months daily; older partitions "will not be refreshed once built."

**Constraints (verbatim):** `incremental` **requires** `update_window`; `incremental` **forbids** `refresh_key.sql` ("incremental refreshes generate their own SQL"); partitioned/incremental rollups **require the time trio** — a `time`-typed `time_dimension` + `granularity` + `partition_granularity`. **This is the one surveyed system besides smelt that structurally requires a time/`event_time`-like column** — the `time_dimension` makes "which recent partitions to refresh" well-defined. Detection: non-incremental polls user `refresh_key.sql`; incremental auto-generates refresh-key SQL keyed on the `time_dimension` + partition boundaries.

Source: [Pre-aggregation reference](https://docs.cube.dev/reference/data-modeling/pre-aggregations).
