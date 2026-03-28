# Spark Backend via PySpark/PyO3 Bridge

**Date**: March 28, 2026
**Status**: Implemented

## Problem

The Spark backend (`smelt-backend-spark`) was a 267-line stub where every `Backend` trait method returned an error. To prove out smelt's multi-backend architecture and enable real Spark/Databricks workloads, we needed a working implementation.

The biggest challenge was **connectivity** — how does a Rust binary talk to a Spark cluster?

## Approach Evaluation

### Option A: Pure Rust via spark-connect-rs
- **Rejected**: Both community (`sjrusso8/spark-connect-rs`) and Apache official (`apache/spark-connect-rust`) crates are explicitly "highly experimental, not for production"
- Requires `protoc` at build time for gRPC
- No Databricks Connect compatibility

### Option B: Databricks SQL REST API
- **Rejected**: Only works with Databricks, not open-source Spark, EMR, or Dataproc
- Too narrow for smelt's multi-backend story

### Option C: PySpark/PyO3 Bridge (chosen)
- PySpark is the most battle-tested Spark client
- Databricks Connect v2 is literally PySpark with extras (`pip install databricks-connect`)
- smelt already had deep PyO3 integration (planner rules, Python model execution)
- Zero-copy Arrow conversion via C Data Interface (`arrow-pyarrow` crate)
- Works everywhere: local Spark, YARN, K8s, Databricks, EMR, Dataproc

## Architecture

```
Rust (smelt-backend-spark)          Python (smelt.spark_adapter)
┌─────────────────────────┐         ┌──────────────────────────┐
│ Backend trait impl      │         │ SparkAdapter             │
│ - SQL generation (DDL)  │──PyO3──>│ - SparkSession lifecycle │
│ - Partition management  │         │ - spark.sql() execution  │
│ - Arrow conversion      │<──────  │ - pyarrow.Table return   │
└─────────────────────────┘         └──────────────────────────┘
```

**Data path**: `spark.sql(query)` → PySpark DataFrame → `.toArrow()` → `pyarrow.Table` → `arrow::pyarrow::FromPyArrow` → Rust `Vec<RecordBatch>` (zero-copy via C Data Interface)

All SQL generation, planning, and orchestration stays in Rust. Python handles only Spark session management and query execution.

## Implementation Details

### Python Adapter (`python/smelt/spark_adapter.py`)
- ~65 lines wrapping PySpark `SparkSession`
- Methods: `execute_sql()` (returns `pyarrow.Table`), `execute_sql_no_result()` (DDL/DML), `table_exists()`, `get_row_count()`, `close()`
- PySpark version detection: `toArrow()` (4.0+) with `toPandas()` fallback (3.x)

### Rust Backend (`crates/smelt-backend-spark/src/lib.rs`)
- `SparkBackend` holds a `Py<PyAny>` reference to the Python adapter
- All Python calls go through `tokio::task::spawn_blocking` + `Python::attach` (GIL safety)
- Same async wrapping pattern as DuckDB backend's `spawn_blocking` for synchronous ops

### Spark-Specific SQL Patterns
| Method | SQL |
|---|---|
| `create_table_as` | `DROP TABLE IF EXISTS {name}; CREATE TABLE {name} AS {sql}` |
| `create_view_as` | `CREATE OR REPLACE VIEW {name} AS {sql}` |
| `ensure_schema` | `CREATE DATABASE IF NOT EXISTS {catalog}.{schema}` |
| `merge_into` | Standard MERGE INTO ... USING ... ON ... WHEN MATCHED/NOT MATCHED |
| `insert_overwrite` | `INSERT OVERWRITE TABLE {name} PARTITION ({col}) {sql}` |

### Feature Flags
- `smelt-cli`: `spark` feature now implies `python` (PySpark requires Python runtime)
- `smelt-backend-spark`: Uses `pyo3` and `arrow` with `pyarrow` feature directly

### pyo3 Upgrade (0.24 → 0.26)
Required because `arrow-pyarrow` 57.x depends on `pyo3 0.26`. Key API changes:
- `Python::with_gil` → `Python::attach`
- `PyObject` → `Py<PyAny>`
- `Py::clone()` → `Py::clone_ref(py)` (requires GIL token)
- `Bound::into()` → `Bound::unbind()` for PyObject conversion

## Configuration

```yaml
# smelt.yml
targets:
  spark_prod:
    type: spark
    connect_url: sc://localhost:15002
    catalog: spark_catalog
    schema: my_schema
```

For Databricks: same config but with Databricks Connect URL and `pip install databricks-connect` instead of `pyspark`.

## Future Work

- **Integration tests**: Run DuckDB integration test suite against local Spark Connect (Docker)
- **Authentication docs**: Document token-based auth, OAuth, instance profiles for Databricks
- **PySpark version matrix**: Test against PySpark 3.x, 4.0+, and Databricks Connect
- **Large result set handling**: Current approach collects all data to driver; document limitation and add streaming option if needed
