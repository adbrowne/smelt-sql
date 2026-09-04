# smelt Examples

This directory contains example models demonstrating smelt capabilities. Unlike `test-workspace`, which is used for editor integration testing and may contain intentionally broken models, this directory contains working examples for testing and demonstration.

## Directory Structure

```
examples/
├── models/              # Model definitions
│   ├── users.sql       # User data (table)
│   ├── events.sql      # Event data (table)
│   └── user_activity.sql  # User activity summary (table)
├── target/             # DuckDB database and artifacts
├── smelt.yml           # Project configuration
├── sources.yml         # Source table definitions
└── setup_sources.sql   # Script to populate source tables
```

## Models

The examples demonstrate a simple user analytics pipeline:

1. **users** - User information from raw source
   - Materialization: table
   - Source: `raw.users`

2. **events** - Event data from raw source
   - Materialization: table
   - Source: `raw.events`

3. **user_activity** - User activity summary using `smelt.ref()`
   - Materialization: table
   - Dependencies: users, events
   - Demonstrates: Multi-table references, aggregation

## Setup

Before running the examples, populate the source tables:

```bash
cd examples
duckdb target/dev.duckdb < setup_sources.sql
```

## Running Examples

### DuckDB (default backend)

```bash
# Run all models
cargo run --bin smelt -- run --project-dir examples/timeseries

# Show query results
cargo run --bin smelt -- run --project-dir examples/timeseries --show-results

# Show compiled SQL
cargo run --bin smelt -- run --project-dir examples/timeseries --verbose

# Validate without executing
cargo run --bin smelt -- run --project-dir examples/timeseries --dry-run
```

## Configuration

### smelt.yml

The project configuration defines one target:

- **dev** (DuckDB): Local development using DuckDB
  - Database: `target/dev.duckdb`
  - Schema: `main`

### sources.yml

Defines source tables from the `raw` schema:
- `raw.users` - User information
- `raw.events` - Event data

## Output Example

```
Project directory: examples
Project: smelt_examples (version 1)
Loaded 2 source tables
Found 3 models

Execution order: 1. users → 2. events → 3. user_activity

Backend: DuckDB
Database: target/dev.duckdb

============================================================
Executing models...
============================================================

▶ Running model: users
  ✓ users (5 rows, 3.614ms)

▶ Running model: events
  ✓ events (10 rows, 2.183ms)

▶ Running model: user_activity
  ✓ user_activity (4 rows, 5.000ms)

============================================================
Summary
============================================================
✓ Executed 3 models successfully
  Total time: 10.797ms
```

## Key Differences from test-workspace

| Feature | examples/ | test-workspace/ |
|---------|-----------|-----------------|
| Purpose | Testing & demonstration | Editor integration testing |
| Models | All working | May include broken models |
| Source tables | Defined in sources.yml | Uses SQL setup script |
| Backend testing | DuckDB | DuckDB only |
| Use case | CLI testing, feature demos | LSP testing, parsing edge cases |

## Notes

- **Parser limitations**: The smelt parser currently doesn't support JOIN syntax. Use comma-separated FROM clauses instead:
  ```sql
  -- ✅ Works
  FROM smelt.ref('users') u,
       smelt.ref('events') e
  WHERE u.user_id = e.user_id

  -- ❌ Not yet supported
  FROM smelt.ref('users') u
  LEFT JOIN smelt.ref('events') e ON u.user_id = e.user_id
  ```

- **Source table setup**: Run `setup_sources.sql` before first execution to populate raw source tables in DuckDB.
