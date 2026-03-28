# Multi-Engine Example Project

**Date**: 2026-03-28
**Status**: Complete

## Context

smelt supports multiple backends (DuckDB, Spark) but lacks an example demonstrating
cross-engine execution where different models run on different engines within the same
project. This example fills that gap by building a realistic clickstream analytics
pipeline that uses Spark for heavy aggregation and DuckDB for business metrics.

## Design

### Pipeline Architecture

```
datagen (100K sessions, 14 days)
  -> raw.sessions (Parquet on disk)
    -> stg_sessions [Spark] (type casting, null handling)
      -> int_visitor_daily [Spark, incremental] (aggregate by visitor+day+country)
        -> mart_daily_metrics [DuckDB, incremental] (daily KPIs)
        -> mart_country_metrics [DuckDB] (country-level rollup)
```

### Cross-Engine Data Transfer

The mart models on DuckDB reference int_visitor_daily on Spark. This is a cross-engine
reference. The planned approach is direct Parquet reads: Spark writes partitioned Parquet
to a warehouse directory, and DuckDB reads from that path directly. No explicit copy step
is needed since DuckDB can read Parquet natively.

### Key Decision: No Copy Step

Rather than materializing Spark output and copying it into DuckDB, DuckDB reads Spark's
Parquet warehouse files directly. This avoids data duplication and simplifies the
orchestration. The planner will resolve `smelt.ref('int_visitor_daily')` to the
appropriate Parquet path when the consuming model runs on a different engine.

## Phases

### Phase 1: Example Skeleton ✅ (March 28, 2026)

- [x] datagen.yaml with clickstream session data
- [x] smelt.yml with dual targets (duckdb_local, spark_docker)
- [x] sources.yml for raw session data
- [x] 4 SQL models across staging/intermediate/marts layers
- [x] docker-compose.yml for Spark Connect
- [x] run.sh orchestration script
- [x] Verified datagen produces correct Parquet output

### Phase 2: Cross-Engine Ref Resolution ✅ (March 28, 2026)

- [x] `find_cross_backend_edges()` detects cross-engine dependencies in LogicalGraph
- [x] `PrintContext.cross_engine_refs` maps model names to `read_parquet()` expressions
- [x] `SqlCompiler.set_cross_engine_refs()` wires cross-engine resolution into compilation
- [x] DuckDB models resolve `smelt.ref('spark_model')` to `read_parquet('{warehouse}/{schema}/{model}/**/*.parquet', hive_partitioning=true)`
- [x] Normal (same-engine) refs continue to resolve to `schema.model`

### Phase 3: Incremental Cross-Engine + Multi-Batch Demo ✅ (March 28, 2026)

- [x] Incremental models work across engine boundaries
- [x] Multi-batch demo runs end-to-end in local mode
- [x] run.sh orchestration handles Spark and DuckDB execution stages
- [x] New Parquet files from subsequent batches are automatically picked up by DuckDB

### Phase 4: Integration Tests + Documentation ✅ (March 28, 2026)

- [x] 7 integration tests in `crates/smelt-cli/tests/multi_engine_test.rs`
  - LogicalGraph cross-engine edge detection (2 tests)
  - SqlCompiler cross-engine ref emission (2 tests)
  - DuckDB Parquet read via cross-engine ref (1 test)
  - End-to-end compile + execute with incremental batches (1 test)
  - Multi-engine example project validation (1 test)
- [x] Plan document updated with results and limitations
- [x] Roadmap updated

## Results

What works end-to-end:

- **Cross-engine ref resolution via direct Parquet**: When a DuckDB model references a Spark model, the compiler emits `read_parquet('{warehouse}/{schema}/{model}/**/*.parquet', hive_partitioning=true)` instead of `schema.model`.
- **DuckDB reads Spark-produced Parquet**: DuckDB natively reads Parquet files written by Spark without any copy or import step.
- **Incremental models work across engine boundaries**: Spark writes new Parquet files per batch; DuckDB re-reads all files on each downstream run.
- **Multi-batch demo runs end-to-end**: The `run.sh` script orchestrates data generation, Spark execution, and DuckDB execution in sequence. Adding new batches of Parquet files is automatically picked up by downstream DuckDB models.

## Known Limitations

- **Full-table Parquet read on every downstream run**: DuckDB reads all Parquet files (`**/*.parquet`) each time, with no partition pruning on the exchange boundary.
- **Local filesystem only**: Parquet paths are local filesystem paths; no S3/GCS/ADLS support for the exchange.
- **Spark simulation in local mode**: The example runs with local Spark or simulated output; Docker is required for real Spark Connect.
- **Type mapping between engines not validated**: Potential edge cases in type coercion between Spark and DuckDB Parquet schemas (e.g., Decimal precision, timestamp timezone handling).
- **No schema validation at exchange boundary**: No compile-time or runtime check that the upstream Parquet schema matches what the downstream model expects.

## Future Work

- **Partition-level reads**: Only read new/changed Parquet partitions instead of full `**/*.parquet` glob. Could use Hive partitioning on the exchange date to filter.
- **Remote storage support**: S3, GCS, ADLS paths for cross-engine Parquet exchange.
- **Schema validation at cross-engine boundaries**: Compare upstream Parquet schema with downstream model's expected input columns and types.
- **Automatic type coercion between engine dialects**: Handle known type mismatches (e.g., Spark STRING vs DuckDB VARCHAR) at the exchange boundary.
- **Streaming exchange for real-time pipelines**: Instead of batch Parquet files, support streaming data exchange between engines.

## Key Files

- `examples/multi_engine/smelt.yml` - Project config with dual targets
- `examples/multi_engine/sources.yml` - Raw session source schema
- `examples/multi_engine/datagen.yaml` - Data generation config
- `examples/multi_engine/models/staging/stg_sessions.sql` - Spark staging
- `examples/multi_engine/models/intermediate/int_visitor_daily.sql` - Spark aggregation
- `examples/multi_engine/models/marts/mart_daily_metrics.sql` - DuckDB daily KPIs
- `examples/multi_engine/models/marts/mart_country_metrics.sql` - DuckDB country rollup
- `examples/multi_engine/docker-compose.yml` - Spark Connect server
- `examples/multi_engine/run.sh` - Orchestration script
- `crates/smelt-cli/tests/multi_engine_test.rs` - Integration tests
