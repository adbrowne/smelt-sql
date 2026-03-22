# Plan: Comprehensive Incremental Model Support

**Date:** 2026-03-22
**Status:** Proposed

## Context

smelt has basic incremental materialization (DELETE+INSERT, March 2026) but lacks the depth needed for production use. Real-world dbt/SQLMesh users consistently hit: late-arriving data, no backfill granularity, testing gaps, schema evolution pain, and orchestration blindspots. smelt's logical/physical separation positions it to solve these structurally rather than with band-aids.

This plan covers: strategy expansion, configuration unification, backfill intelligence, state tracking, schema evolution, orchestrator integration, testing, doc cleanup, and user documentation outlines.

---

## 1. Current State

### What Works
- DELETE+INSERT strategy with partition-based management
- Hour/Day/Week(configurable start)/Month granularity
- Safety checks: window functions, HAVING, LIMIT, subqueries, non-deterministic, DISTINCT
- CLI: `--event-time-start` / `--event-time-end` (required, ISO 8601)
- Time filter injection into WHERE clauses via AST transformer
- DuckDB backend: `delete_partitions()` + `insert_into_from_query()`
- YAML frontmatter + smelt.yml config with precedence
- Optimizer detection validates partition_column in SELECT/GROUP BY

### Known Issues to Fix
1. **Duplicate IncrementalConfig**: `smelt-core/src/config.rs` (no safety_overrides) vs `smelt-optimizer/src/types.rs` (has safety_overrides, no `enabled`)
2. **Three execution paths** in main.rs (optimizer+incremental, incremental-only, legacy) — should be one
3. **ROADMAP Phase 6** still listed as "Future" despite basic incremental being complete
4. **DESIGN.md annotation syntax** (`-- @materialize: incremental`) never implemented; YAML frontmatter is the real surface — doc should clarify
5. **No lookback/temporal analysis** despite DESIGN.md mentioning lookback_days
6. **Granularity conversion boilerplate** in main.rs (lines 669-683) mapping smelt-core Granularity to optimizer Granularity — unnecessary with unified type

### Key Files
| File | Role |
|------|------|
| `crates/smelt-core/src/config.rs` | IncrementalConfig, Granularity, Weekday |
| `crates/smelt-core/src/metadata.rs` | Frontmatter extraction |
| `crates/smelt-optimizer/src/types.rs` | Duplicate IncrementalConfig + SafetyOverrides |
| `crates/smelt-optimizer/src/rules/incremental.rs` | Detection, validation, 21+ tests |
| `crates/smelt-cli/src/transformer.rs` | `inject_time_filter()`, TimeRange |
| `crates/smelt-cli/src/executor.rs` | `execute_model_incremental()`, `execute_plan_incremental()` |
| `crates/smelt-cli/src/main.rs` | CLI dispatch, `generate_partition_values()` |
| `crates/smelt-backend/src/lib.rs` | Backend trait |
| `crates/smelt-backend-duckdb/src/lib.rs` | DuckDB impl |
| `crates/smelt-dialect/src/dialect.rs` | BackendCapabilities (already has `supports_merge`) |

---

## 2. Competitive Landscape Summary

### dbt Pain Points (from real users)
- **Late-arriving data**: Fundamental unsolved problem; lookback is a band-aid
- **Backfill**: Only full-refresh or one-at-a-time incremental — no middle ground
- **Testing**: CI tests full-refresh; production runs incremental; bugs hide until prod
- **is_incremental()**: Two code paths (full vs incremental) with subtle divergence bugs
- **Schema changes**: `on_schema_change` config is complex and incomplete
- **No state**: Stateless — skipped runs create silent data gaps
- **BigQuery cost**: Incremental merges trigger full table scans unexpectedly

### SQLMesh Advantages
- Declarative interval-based processing (no `is_incremental()`)
- `@start_ds`/`@end_ds` macros for clean temporal boundaries
- Interval tracking: knows which ranges each model version has processed
- Plan-based changes: breaking vs non-breaking classification with targeted backfill
- Lookback parameter for late-arriving data
- SCD Type 2 built-in

### Dagster Alignment (vs Airflow)
- Asset-centric model maps directly to smelt models-as-assets
- Native partition support for time-windowed incremental
- Auto-materialization policies
- 2x developer productivity vs Airflow
- Airflow 3.0 added Data Assets but still task-centric at core

### smelt's Structural Advantages
- Parser understands SQL semantics — can **prove** batch safety
- Logical/physical separation — strategies are rewrite rules, not macros
- Backends own computational state — no watermark consistency bugs
- Type system + column lineage — precise invalidation

---

## 3. Implementation Phases

### Phase 1: Configuration Unification & Cleanup

**Goal:** Single IncrementalConfig, one execution path, doc fixes.

#### 1a. Merge IncrementalConfig types

Single canonical type in `smelt-core/src/config.rs`:

```rust
pub struct IncrementalConfig {
    pub enabled: bool,
    pub event_time_column: String,
    pub partition_column: String,
    pub granularity: Granularity,
    pub strategy: IncrementalStrategy,        // NEW — default: DeleteInsert
    pub unique_key: Vec<String>,              // NEW — required for Merge
    pub safety_overrides: IncrementalSafetyOverrides, // MOVED from optimizer
}

pub enum IncrementalStrategy {
    DeleteInsert,  // default — current behavior
    Merge,         // UPSERT via unique_key
    Append,        // insert-only, no dedup
    InsertOverwrite, // replace entire partitions
}
```

**Note: No explicit lookback config on incremental models.** Two concerns are handled separately:
1. **Temporal dependencies** — inferred from the SQL AST (see Phase 3)
2. **Data latency** — declared on upstream sources/models, not on the consumer (see Phase 3b)

Move `IncrementalSafetyOverrides` from `smelt-optimizer/src/types.rs` to `smelt-core/src/config.rs`. Update optimizer to import from core.

**Data latency** is a property of the table that produces the data, not the model that reads it:

```yaml
# sources.yml — latency declared per column on the producing table
sources:
  - name: raw
    tables:
      - name: transactions
        columns:
          - name: event_time
            type: TIMESTAMP
            data_latency: { count: 3, unit: days }
          - name: ingestion_time
            type: TIMESTAMP
            data_latency: { count: 0, unit: hours }

# models can also declare per-column latency (propagates to downstream)
---
name: events_cleaned
columns:
  event_time:
    data_latency: { count: 3, unit: days }
---
```

When building the execution plan for a downstream model, smelt traces the model's `event_time_column` to the upstream source column and resolves its `data_latency`. Different columns on the same table can have different latencies — a model filtering on `event_time` gets 3-day buffer while one filtering on `ingestion_time` gets none.

Move `IncrementalSafetyOverrides` from `smelt-optimizer/src/types.rs` to `smelt-core/src/config.rs`. Update optimizer to import from core.

**Files:** `smelt-core/src/config.rs`, `smelt-optimizer/src/types.rs`, `smelt-optimizer/src/graph.rs`, `smelt-core/src/metadata.rs`

#### 1b. Unify execution paths

Replace three dispatch cases in main.rs with:

```rust
enum ModelExecution {
    FullRefresh,
    Incremental {
        config: IncrementalConfig,
        time_range: TimeRange,
        plan_steps: Option<Vec<ExecutionStep>>, // from optimizer (e.g., cube split)
    },
}
```

Single code path: build `ModelExecution` from either optimizer output or config, then execute uniformly.

**Files:** `smelt-cli/src/main.rs`, `smelt-cli/src/executor.rs`

#### 1c. Doc cleanup

- **ROADMAP.md**: Mark Phase 6 as partially complete. Add reference to this plan for "Phase 6b: Advanced Incremental."
- **DESIGN.md**: Add note that `-- @materialize` annotation syntax is a future option; YAML frontmatter is the current config surface.

#### 1d. Granularity improvements

Current granularities are good (Hour, Day, Week{week_start}, Month). Add:

```rust
pub enum Granularity {
    Hour,
    Day,
    Week { week_start: Weekday },
    Month,
    Quarter,   // NEW — useful for fiscal reporting
    Year,      // NEW — rare but needed for annual aggregations
}
```

Update `generate_partition_values()` for Quarter and Year.

---

### Phase 2: Strategy Expansion

**Goal:** MERGE, APPEND, INSERT_OVERWRITE alongside existing DELETE+INSERT.

#### 2a. Backend trait extensions

Add to `smelt-backend/src/lib.rs`:

```rust
async fn merge_into(
    &self, schema: &str, table: &str,
    source_sql: &str, unique_key: &[String],
) -> Result<(), BackendError>;

async fn insert_overwrite(
    &self, schema: &str, table: &str,
    sql: &str, partition: &PartitionSpec,
) -> Result<(), BackendError>;
```

Update `execute_model_incremental()` default impl to dispatch on strategy.

#### 2b. DuckDB implementations

- **MERGE**: `MERGE INTO schema.table USING (source_query) ON key_match WHEN MATCHED THEN UPDATE SET * WHEN NOT MATCHED THEN INSERT *`
- **APPEND**: `INSERT INTO schema.table SELECT ... FROM (source_query)`
- **INSERT_OVERWRITE**: DuckDB lacks native INSERT OVERWRITE. Implement as DELETE where partition_column IN (SELECT DISTINCT partition_column FROM source) + INSERT.

#### 2c. Optimizer validation

Update `smelt-optimizer/src/rules/incremental.rs`:
- MERGE: require `unique_key` non-empty, validate key columns exist in SELECT
- APPEND: relax partition_column requirement
- INSERT_OVERWRITE: require partition_column in SELECT (same as DELETE+INSERT)

#### 2d. Strategy compatibility matrix

```
Strategy         | unique_key | partition_col | DuckDB | Spark  | PostgreSQL
delete_insert    | No         | Required      | Yes    | Yes    | Yes
merge            | Required   | Optional      | Yes    | Delta  | PG 15+
append           | No         | No            | Yes    | Yes    | Yes
insert_overwrite | No         | Required      | Emul.  | Native | No
```

**Files:** `smelt-backend/src/lib.rs`, `smelt-backend/src/types.rs`, `smelt-backend-duckdb/src/lib.rs`, `smelt-optimizer/src/rules/incremental.rs`

---

### Phase 3: Temporal Dependency Inference & Data Latency

**Goal:** Automatically determine how much context each query needs (from the AST), and separately handle late-arriving data (from config). These are two orthogonal concerns:

1. **Temporal dependencies** (inferred from SQL): "this query needs 6 prior days to compute correctly" — e.g., window functions, self-joins with date offsets, LAG/LEAD
2. **Data latency** (configured): "upstream events can arrive up to 3 days late" — operational knowledge about the pipeline, not the query

#### 3a. AST-Based Temporal Dependency Analysis

New analysis pass in `smelt-optimizer` that examines the query AST to infer how much historical context is needed beyond the requested time range:

```rust
pub struct TemporalDependency {
    /// How many periods of prior data the query needs
    pub lookback: Duration,
    /// How many periods of future data the query needs (rare — LEAD, lookahead joins)
    pub lookahead: Duration,
    /// Where the dependency was detected (for explain output)
    pub sources: Vec<TemporalSource>,
}

pub enum TemporalSource {
    WindowFrame { function: String, frame: String },  // "SUM OVER ROWS BETWEEN 6 PRECEDING"
    LagLead { function: String, offset: u32 },        // "LAG(col, 3)"
    JoinOffset { interval: String },                  // "ON a.day = b.day - INTERVAL '1 day'"
    WhereOffset { interval: String },                 // "WHERE ts >= day - INTERVAL '3 days'"
    Unbounded { reason: String },                     // correlated subquery with no temporal bound
}
```

Recognizable patterns:

| SQL Pattern | Inferred dependency |
|-------------|-------------------|
| Simple `GROUP BY` with partition col | 0 (partition-local) |
| `ROWS BETWEEN N PRECEDING AND CURRENT ROW` | lookback = N periods |
| `RANGE BETWEEN INTERVAL 'N days' PRECEDING` | lookback = N days |
| `LAG(col, N)` | lookback = N periods |
| `LEAD(col, N)` | lookahead = N periods |
| `JOIN ... ON a.date = b.date - INTERVAL 'N'` | lookback = N |
| `JOIN ... ON a.date = b.date + INTERVAL 'N'` | lookahead = N |
| `WHERE col >= partition_date - INTERVAL 'N'` | lookback = N |
| Correlated subquery, no temporal bound | **Unbounded** — warn, require override or full refresh |
| No recognizable pattern | Conservative: warn, suggest explicit annotation |

When multiple patterns exist, take the **max** across all detected dependencies.

For **Unbounded** cases, smelt should error with a clear message:
```
Model 'user_lifetime_value': detected unbounded temporal dependency
  (correlated subquery at line 12 scans full history for each user).
  Options:
    1. Add a temporal bound to the subquery
    2. Use `safety_overrides.allow_unbounded_lookback: true` and accept full-refresh-only
    3. Restructure as a separate model
```

**Files:** NEW `crates/smelt-optimizer/src/analysis/temporal.rs`

#### 3b. Data Latency (Property of Upstream, Not Downstream)

Data latency is declared on the table that *produces* the data — sources or models — not on the consumer. Crucially, latency is **per-column** because different columns on the same table can have very different arrival characteristics:

- `ingestion_time` — set when data lands in the warehouse (near-zero latency)
- `event_time` — when the event actually happened (could be days earlier for mobile offline sync, batch uploads, etc.)

```yaml
# sources.yml — latency declared per column
sources:
  - name: raw
    tables:
      - name: transactions
        columns:
          - name: event_time
            type: TIMESTAMP
            data_latency: { count: 3, unit: days }   # mobile events arrive up to 3 days late
          - name: ingestion_time
            type: TIMESTAMP
            data_latency: { count: 0, unit: hours }   # set on warehouse arrival
          - name: amount
            type: DECIMAL
      - name: clicks
        columns:
          - name: click_time
            type: TIMESTAMP
            data_latency: { count: 1, unit: hours }   # near-real-time stream

# model frontmatter — latency on output columns propagates downstream
---
name: events_cleaned
columns:
  event_time:
    data_latency: { count: 3, unit: days }   # inherited from source, or explicitly declared
---
```

When smelt builds the execution plan for a downstream incremental model, it resolves latency by matching the model's `event_time_column` to the upstream column:

1. The downstream model declares `event_time_column: event_time`
2. smelt traces `event_time` through the query to its upstream source column(s)
3. The relevant `data_latency` is the latency of those specific upstream columns
4. If multiple upstream columns contribute, take the **max**

This means a model filtering on `event_time` (3-day latency) gets a 3-day buffer, while a different model filtering on `ingestion_time` (0 latency) on the **same source table** gets no buffer at all.

`unit: partitions` means "N granularity periods of the consuming model," so a weekly model reading a source with `count: 1, unit: partitions` gets 1 week of latency buffer.

**Files:** `crates/smelt-core/src/config.rs` (LatencyWindow on column definitions), `crates/smelt-db/src/lib.rs` (latency resolution via column lineage)

#### 3c. Effective Window Computation

The total window applied to each run is:

```
effective_lookback  = max(ast_inferred_lookback, data_latency)
effective_lookahead = ast_inferred_lookahead  (data_latency doesn't affect lookahead)
```

Example scenarios:

```
Model: daily_revenue (GROUP BY day, event_time_column=event_time, source event_time has 3-day latency)
  AST temporal dependency: 0 days (partition-local)
  Upstream column latency: 3 days (from raw.transactions.event_time)
  Effective lookback: 3 days

Model: daily_ingestion_stats (GROUP BY day, event_time_column=ingestion_time, same source but 0 latency)
  AST temporal dependency: 0 days (partition-local)
  Upstream column latency: 0 (from raw.transactions.ingestion_time)
  Effective lookback: 0 days

Model: user_rolling_7d (window function with 6 PRECEDING, reads events_cleaned.event_time with 3-day latency)
  AST temporal dependency: 6 days (from ROWS BETWEEN 6 PRECEDING)
  Upstream column latency: 3 days (from events_cleaned.event_time)
  Effective lookback: 6 days (AST wins — already covers latency)

Model: day_over_day_change (self-join with 1-day offset)
  AST temporal dependency: 1 day (from JOIN ON a.day = b.day + INTERVAL '1 day')
  Upstream column latency: 0
  Effective lookback: 1 day
```

#### 3d. Explain Output

`smelt explain` shows both components transparently:

```
$ smelt explain user_rolling_7d

user_rolling_7d:
  Strategy: delete_insert (day granularity)
  Temporal dependencies:
    lookback: 6 days (from: SUM OVER ROWS BETWEEN 6 PRECEDING AND CURRENT ROW, line 5)
    lookahead: 0
  Upstream column latency: 3 days (from: events_cleaned.event_time)
  Effective window: lookback=6 days, lookahead=0
  Batch safety: bounded (needs 6-day context per batch)
```

#### 3e. Implementation

Modify `generate_partition_values()` in main.rs: before generating, compute effective start/end by applying the combined window. The transformer's `inject_time_filter()` receives the already-adjusted TimeRange — no changes needed there.

For lookahead (rare but needed for LEAD), extend the end date:
```
Requested range:     [2026-03-20, 2026-03-22)
Lookback: 6 days
Lookahead: 1 day
Filter range:        [2026-03-14, 2026-03-23)   ← what goes in WHERE clause
Partition DELETE:    [2026-03-20, 2026-03-22)   ← only delete/replace requested partitions
```

Note: the filter range is wider than the partition range. We fetch extra context rows for the query to compute correctly, but only write/replace the requested partitions.

**Files:**
- NEW `crates/smelt-optimizer/src/analysis/temporal.rs` — AST analysis
- `crates/smelt-core/src/config.rs` — `LatencyWindow` type (from Phase 1a)
- `crates/smelt-cli/src/main.rs` — apply combined window to partition generation
- `crates/smelt-cli/src/transformer.rs` — may need separate filter range vs partition range

---

### Phase 4: Backfill Intelligence

**Goal:** Smart backfill that picks the right execution shape — not just "one query per partition" or "full refresh."

This is where smelt's semantic understanding pays off. dbt gives you two options: full refresh (nuclear) or one incremental run (slow). smelt should offer a spectrum.

#### The Backfill Spectrum

```
Most efficient                                              Least efficient
     |                                                             |
Single query    Large chunks     Small chunks    Per-partition   Full refresh
(batch-safe)    (30-day batches) (7-day batches) (1 per period)  (DROP+CREATE)
```

#### Batch Safety Analysis

Batch safety is a direct consequence of Phase 3's temporal dependency analysis:

- **Temporal dependency = 0** (partition-local): Fully batch-safe — single query for any range
- **Temporal dependency = bounded N**: Safe in chunks, but each chunk needs N extra context rows. A backfill query covering range [start, end) fetches [start - N, end) but only writes [start, end).
- **Temporal dependency = unbounded**: Must process per-partition or full refresh

The existing safety checks in `smelt-optimizer/src/rules/incremental.rs` already detect most unsafe patterns. Extend with:

```rust
pub enum BatchSafety {
    /// Single query for any range — aggregations are partition-local
    FullyBatchSafe,
    /// Safe for bounded chunks — e.g., window functions need context but bounded
    BoundedSafe { max_chunk_days: u32, reason: String },
    /// Must process per-partition — cross-partition dependencies
    PerPartitionOnly { reason: String },
}

pub fn analyze_batch_safety(model: &ModelInfo) -> BatchSafety
```

#### Backfill Command

```bash
# Backfill a date range — smelt picks optimal batch size
smelt backfill daily_revenue --start 2025-01-01 --end 2026-01-01

# Override batch size (days)
smelt backfill daily_revenue --start 2025-01-01 --end 2026-01-01 --batch-size 30

# Force per-partition (override batch safety analysis)
smelt backfill daily_revenue --start 2025-01-01 --end 2026-01-01 --per-partition

# Cascade to downstream models
smelt backfill daily_revenue --start 2025-01-01 --end 2026-01-01 --cascade

# Dry run — show what would execute
smelt backfill daily_revenue --start 2025-01-01 --end 2026-01-01 --dry-run
```

#### Default behavior

1. Run `analyze_batch_safety()` on the model
2. If `FullyBatchSafe`: execute the entire range as a **single** DELETE+INSERT (or MERGE, etc.)
3. If `BoundedSafe { max_chunk_days: 30 }`: split into 30-day chunks, one query per chunk
4. If `PerPartitionOnly`: split into per-granularity chunks (one per day/week/month)
5. User can always override with `--batch-size N` or `--per-partition`

This means a 365-day backfill of a batch-safe daily model runs **1 query** (DELETE WHERE date BETWEEN ... AND ... + INSERT), not 365 queries.

#### Weekly/Monthly backfill

For weekly models, a 1-year backfill = ~52 chunks at worst (per-partition), or 1 query if batch-safe. The partition values generated respect week boundaries via the existing `week_start` config.

For monthly models, 1-year backfill = 12 chunks at worst, or 1 query if batch-safe.

#### Downstream cascade

When `--cascade` is specified:
1. Traverse dependency graph (`smelt-cli/src/graph.rs`) to find downstream models
2. For each downstream incremental model: backfill the same time range
3. For non-incremental downstream models: trigger full refresh
4. Execution follows topological order

#### Files

- **NEW** `crates/smelt-cli/src/backfill.rs` — batch generation, safety-to-batch-size mapping, cascade
- `crates/smelt-optimizer/src/rules/incremental.rs` — add `analyze_batch_safety()`
- `crates/smelt-cli/src/main.rs` — add `Backfill` subcommand

---

### Phase 5: Operational Metadata & Run History

**Goal:** Track what smelt has done (operational metadata, NOT computational state).

Per DESIGN.md: smelt tracks run history, schema lineage, DAG deps, deployed versions. Backends own watermarks, offsets, partition data.

#### Run Manifests

Store in `.smelt/runs/` as JSON:

```json
{
  "run_id": "20260322-143022-abc123",
  "started_at": "2026-03-22T14:30:22Z",
  "completed_at": "2026-03-22T14:31:05Z",
  "models": {
    "daily_revenue": {
      "strategy": "delete_insert",
      "time_range": { "start": "2026-03-20", "end": "2026-03-22" },
      "partitions_updated": ["2026-03-20", "2026-03-21"],
      "row_count": 1542,
      "duration_ms": 230,
      "batch_safety": "fully_batch_safe"
    }
  }
}
```

#### Interval Tracking

Store in `.smelt/intervals.json`:

```json
{
  "daily_revenue": {
    "model_hash": "sha256:abc...",
    "covered_intervals": [
      { "start": "2026-01-01", "end": "2026-03-22" }
    ]
  }
}
```

Enables:
- **Gap detection**: `smelt status` warns about uncovered intervals
- **Auto mode**: `smelt run --auto` processes only uncovered intervals since last run
- **Model change detection**: When model_hash changes, intervals can be selectively invalidated

#### New Crate: `smelt-state`

```
crates/smelt-state/
  src/
    lib.rs          -- RunManifest, IntervalStore trait
    intervals.rs    -- Interval tracking, gap detection, merge logic
    history.rs      -- Run history queries
    file_store.rs   -- JSON file-backed implementation
```

#### CLI additions

```bash
smelt run --auto              # Process uncovered intervals
smelt status                  # Show interval coverage + gaps
smelt history [model_name]    # Show run history
```

#### Design principle

The interval store is **advisory, not authoritative**. Backends are truth for what data exists. If the interval store is deleted, smelt keeps working — you just lose auto-detection and must specify ranges manually. `--full-refresh` is always the escape hatch.

**Files:** NEW `crates/smelt-state/`, `crates/smelt-cli/src/main.rs`

---

### Phase 6: Schema Evolution

**Goal:** ALTER TABLE + targeted backfill instead of full refresh when schemas change.

#### Schema tracking

Store deployed schemas in `.smelt/schemas/{model_name}.json`:

```json
{
  "model": "daily_revenue",
  "version": 3,
  "deployed_at": "2026-03-22T14:30:00Z",
  "model_hash": "sha256:...",
  "columns": [
    { "name": "order_date", "type": "DATE", "nullable": false },
    { "name": "total", "type": "DECIMAL(10,2)", "nullable": true }
  ]
}
```

#### Change detection

Compare inferred schema (from `smelt-db` type inference) against deployed schema:

```rust
enum SchemaChange {
    AddColumn { name: String, data_type: DataType, nullable: bool },
    RemoveColumn { name: String },
    ChangeType { name: String, from: DataType, to: DataType },
    NoChange,
}
```

#### Migration strategies

| Change | Action | Incremental impact |
|--------|--------|-------------------|
| Add nullable column | ALTER TABLE ADD COLUMN + backfill UPDATE | Continue incremental (new inserts include column) |
| Add NOT NULL column | Full refresh required | Intervals invalidated |
| Remove column | ALTER TABLE DROP (requires `--allow-column-removal`) | Continue incremental |
| Widen type (INT->BIGINT) | ALTER TABLE ALTER TYPE | Continue incremental |
| Narrow type | Full refresh (data loss risk) | Intervals invalidated |

#### Safety

- Wrap in transactions where supported (check `BackendCapabilities`)
- Never auto-drop columns — require explicit flag
- `--dry-run` shows migration plan without executing

**Files:** NEW `crates/smelt-state/src/schema_tracking.rs`, NEW `crates/smelt-cli/src/migration.rs`, `crates/smelt-cli/src/executor.rs`, `crates/smelt-backend/src/lib.rs`

---

### Phase 7: Testing Infrastructure

**Goal:** Test incremental behavior directly, not just full refresh.

#### Incremental correctness test harness

```rust
/// Runs model both ways and compares via EXCEPT
async fn assert_incremental_correct(
    backend: &dyn Backend,
    model: &ModelInfo,
    time_ranges: &[TimeRange],  // run incrementally over these ranges
) -> Result<()> {
    // 1. Full refresh -> baseline table
    // 2. Run incremental for each time range in sequence
    // 3. Compare: SELECT * FROM baseline EXCEPT SELECT * FROM incremental (both directions)
    // 4. Assert no differences
}
```

#### Test scenarios per strategy

1. First run (table doesn't exist) -> CREATE
2. Subsequent run (exists) -> incremental strategy
3. Overlapping ranges -> verify idempotency
4. Late-arriving data -> verify lookback captures
5. Schema change -> verify migration + continued incremental
6. Backfill batch-safe model -> verify single-query produces same result as per-partition
7. Backfill non-batch-safe model -> verify chunked execution is correct

#### Property tests

Extend `crates/smelt-db/tests/` with generators for incremental-eligible queries (must have GROUP BY + partition column). Run both full-refresh and incremental against DuckDB, compare results.

#### Test structure

```
crates/smelt-cli/tests/incremental/
    strategies.rs        -- DELETE+INSERT, MERGE, APPEND, INSERT_OVERWRITE
    lookback.rs          -- Late-arriving data
    backfill.rs          -- Batch subdivision + single-query backfill
    schema_evolution.rs  -- Schema change + continued incremental
    intervals.rs         -- Gap detection, auto mode
```

---

### Phase 8: Orchestrator Integration (Architecture)

**Goal:** Define how smelt integrates with Dagster (primary) and Airflow (secondary). Detailed plugin API design deferred to a separate plan.

#### Core enabler: `smelt explain --json`

New CLI command that outputs model graph + config as JSON for orchestrator consumption:

```bash
smelt explain --json --project-dir ./
```

```json
{
  "models": {
    "daily_revenue": {
      "dependencies": ["orders"],
      "materialization": "table",
      "incremental": {
        "strategy": "delete_insert",
        "granularity": "day",
        "partition_column": "order_date",
        "batch_safety": "fully_batch_safe"
      },
      "tags": ["revenue", "daily"],
      "owner": "analytics-team"
    }
  },
  "execution_order": ["orders", "daily_revenue"]
}
```

#### Dagster integration (primary — strong alignment)

Architecture: Python package `smelt-dagster` that:
1. Calls `smelt explain --json` at asset definition time
2. Creates one `@asset` per smelt model
3. Maps `granularity` to `TimeWindowPartitionsDefinition`
4. Maps `smelt.ref()` deps to Dagster asset dependencies
5. At runtime: calls `smelt run --select model --event-time-start X --event-time-end Y`
6. Partition-aware: Dagster passes partition time window, smelt processes it

Why Dagster fits:
- Asset = model (semantic alignment)
- Partitions = time windows (direct mapping)
- Auto-materialization can drive incremental runs
- Backfill UI maps directly to `smelt backfill`

#### Airflow integration (secondary — weaker fit)

Architecture: Python package `smelt-airflow` that:
1. Calls `smelt explain --json` to discover models
2. Generates DAG with one BashOperator per model
3. Passes `{{ ds }}` / `{{ next_ds }}` as event-time-start/end
4. Task dependencies from model refs

Why weaker fit:
- Task-centric, not asset-centric
- No native partition concept (must build from macros)
- DAG reload overhead scales poorly
- But: many teams already use Airflow, so worth supporting

#### Standalone mode (current)

The CLI with `--auto` (from Phase 5) serves as a lightweight standalone scheduler for dev/small deployments.

**Files:** `crates/smelt-cli/src/main.rs` (Explain subcommand), NEW `crates/smelt-cli/src/explain.rs`

---

## 4. Sequencing & Dependencies

```
Phase 1: Config Unification & Cleanup
  |
Phase 2: Strategy Expansion          Phase 3: Lookback
  |                                    |
Phase 4: Backfill Intelligence  <--- (needs strategies + lookback)
  |
Phase 5: Operational Metadata   (can start after Phase 1)
  |
Phase 6: Schema Evolution       (needs state infrastructure from Phase 5)
Phase 7: Testing                (ongoing, each phase adds tests)
Phase 8: Orchestrator Integration (needs explain command, can start after Phase 4)
```

Each phase is independently valuable:
- **Phase 1 alone**: Cleaner codebase, new config fields available, doc fixes
- **+Phase 2**: Multiple strategies for different use cases
- **+Phase 3**: Late-arriving data handled automatically
- **+Phase 4**: Smart backfill — the biggest user-facing improvement
- **+Phase 5**: Self-managing incremental with gap detection
- **+Phase 6**: Schema changes without downtime
- **+Phase 8**: Production orchestration

---

## 5. Doc Cleanup Checklist

- [ ] ROADMAP.md: Fix Phase 6 status (basic incremental complete, link to this plan for advanced)
- [ ] DESIGN.md: Note that `-- @materialize` annotation is deferred; YAML frontmatter is current surface
- [ ] DESIGN.md: Reconcile `lookback_days` references with new approach (AST-inferred temporal deps + `data_latency` config)
- [ ] Unify IncrementalConfig (eliminate smelt-optimizer duplicate)
- [ ] Remove Granularity conversion boilerplate in main.rs (lines 669-683)
- [ ] Clean up triple execution path in main.rs

---

## 6. User Documentation Outlines

### 6.1 Incremental Models Guide

**Sections:**
1. **What are incremental models?** — Process only new/changed data instead of full table rebuilds
2. **Quick start** — Add `incremental: { enabled: true, event_time_column, partition_column }` to frontmatter
3. **How it works** — smelt injects WHERE filter + uses DELETE+INSERT (or other strategy). Diagram showing: original SQL -> filter injection -> partition delete -> insert
4. **Choosing a strategy** — Decision tree:
   - Immutable time-series events -> `delete_insert` (default)
   - Dimension tables with updates -> `merge` (requires `unique_key`)
   - Append-only logs -> `append`
   - Large partitioned tables -> `insert_overwrite`
5. **Configuration reference** — Full YAML spec with all fields, defaults, examples
6. **Safety checks** — What smelt validates (window functions, HAVING, etc.) and how to override
7. **What you DON'T write** — No `is_incremental()`, no conditional logic. smelt handles it.

### 6.2 Granularity Guide

**Sections:**
1. **Available granularities** — Hour, Day, Week (configurable start day), Month, Quarter, Year
2. **Choosing granularity** — Match your data's natural time grain. Examples:
   - Clickstream -> Hour or Day
   - Financial reporting -> Day or Week (fiscal week start)
   - Monthly KPIs -> Month
3. **Weekly models** — Configuring `week_start: monday` (or any day). Partition values align to week boundaries.
4. **Interaction with lookback** — `unit: partitions` means "N periods at your granularity"

### 6.3 Temporal Dependencies & Data Latency Guide

**Sections:**
1. **Two different problems** — Temporal dependencies (query needs prior data to compute correctly) vs data latency (upstream data arrives late). These are orthogonal.
2. **Temporal dependencies (automatic)** — smelt analyzes your SQL to detect window functions, LAG/LEAD, self-joins with date offsets, and other patterns that require historical context. No config needed — it's inferred from the query.
3. **What smelt detects** — Table of patterns: `ROWS BETWEEN N PRECEDING`, `LAG(col, N)`, `JOIN ON a.day = b.day - INTERVAL '1 day'`, etc.
4. **Data latency (declared on upstream, per-column)** — Latency belongs on the source/model that produces the data, attached to specific columns. A table might have `event_time` with 3-day latency and `ingestion_time` with near-zero. smelt matches the downstream model’s `event_time_column` to the upstream column’s latency.
5. **How they combine** — Effective window = max(temporal dependency, resolved upstream column latency). Diagram showing filter range (wider) vs partition range (only requested partitions).
6. **Lookahead** — Rare but supported: LEAD functions need future context. smelt extends the filter end date while still only writing requested partitions.
7. **Explain output** — `smelt explain model_name` shows both components transparently: what the AST requires, what upstream latency contributes (and from which column/source), and the resulting effective window.
8. **Unbounded dependencies** — What happens when smelt can’t determine a bound (correlated subqueries, etc.) and your options.
9. **Latency propagation** — If source A’s `event_time` has 3-day latency and model B reads A, model B’s output inherits that. Downstream models of B see the cumulative latency.

### 6.4 Backfill Guide

**Sections:**
1. **When to backfill** — New model, bug fix, schema change, data correction
2. **Basic backfill** — `smelt backfill model --start X --end Y`
3. **How smelt picks batch size** — Batch safety analysis: if your model's aggregations are partition-local, smelt runs one query for the entire range. Otherwise it chunks intelligently.
4. **Overriding batch size** — `--batch-size 30` for 30-day chunks, `--per-partition` for one-per-period
5. **Downstream cascade** — `--cascade` reprocesses all downstream models
6. **Backfill vs full refresh** — Backfill reprocesses a range incrementally (preserves data outside the range). Full refresh drops and recreates.
7. **Examples:**
   - Backfill a batch-safe daily model for 1 year: 1 query
   - Backfill a weekly model with window functions: ~52 queries (per-partition)
   - Backfill with cascade to 3 downstream models

### 6.5 Schema Evolution Guide

**Sections:**
1. **Adding a column** — smelt detects the new column, runs ALTER TABLE + backfill UPDATE
2. **Removing a column** — Requires `--allow-column-removal` flag
3. **Changing types** — Widening (INT->BIGINT) is automatic; narrowing requires full refresh
4. **How it interacts with incremental** — New runs include the new column; existing data gets backfill UPDATE
5. **When full refresh is required** — NOT NULL additions, type narrowing, fundamental query changes

### 6.6 Orchestrator Integration Guide

**Sections:**
1. **Standalone mode** — `smelt run --auto` for dev/small deployments
2. **Dagster** — Asset discovery, partition mapping, auto-materialization, backfill via Dagster UI
3. **Airflow** — DAG generation, template variable mapping, task dependencies
4. **Custom integration** — Use `smelt explain --json` to build your own integration
5. **Which to choose** — Dagster for new projects (better semantic fit); Airflow if you already have it

### 6.7 Migration from dbt Guide

**Sections:**
1. **Concept mapping table:**

   | dbt | smelt |
   |-----|-------|
   | `{{ config(materialized='incremental') }}` | `incremental: { enabled: true }` |
   | `{% if is_incremental() %}` | Not needed — automatic |
   | `unique_key` | `incremental.unique_key` |
   | `strategy: merge` | `incremental.strategy: merge` |
   | `on_schema_change` | Automatic schema evolution |
   | `dbt run --full-refresh` | `smelt run --full-refresh` |
   | `dbt run` (incremental) | `smelt run --event-time-start X --end Y` or `smelt run --auto` |
   | N/A | `smelt backfill model --start X --end Y` |

2. **What you can delete** — All `{% if is_incremental() %}` blocks, all conditional WHERE clauses
3. **What changes** — Your model becomes pure SQL; incrementalization config moves to frontmatter
4. **What's new** — Backfill command, batch safety analysis, lookback windows, auto mode

---

## 7. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| MERGE SQL varies across backends | Use `BackendCapabilities::supports_merge`; generate backend-specific SQL |
| Interval store corruption | Advisory only — `--full-refresh` always works; store is deletable |
| Schema migration breaks data | Wrap in transactions; require `--allow-column-removal`; dry-run first |
| Batch safety analysis false positive | Conservative default (per-partition); user can override with `--batch-size` |
| Backfill cascade causes cascade failure | `--dry-run` shows full plan; batch subdivision limits blast radius |

---

## 8. Verification

After each phase, verify:
1. `cargo fmt --all && cargo clippy --all-targets` — clean
2. `cargo test` — all existing tests pass
3. Phase-specific integration tests pass
4. `smelt run` on test-workspace with daily_revenue model works for both full-refresh and incremental
5. Backfill: verify single-query backfill for batch-safe model produces same result as per-partition
