---
feature: datagen
status: experimental
last_reviewed: 2026-05-05
owners: [andrew]
---

# Datagen

> **What this is.** A normative spec for `smelt-datagen` — the deterministic test-data generation tool. Covers the YAML config format, generator types, entity pools, foreign keys, partitioning, and determinism guarantee.

## Surface

### CLI

```
smelt-datagen [OPTIONS]

  --config <PATH>          YAML config file (required)
  -o, --output <PATH>      Output directory override [default: from config]
  -s, --seed <SEED>        Random seed override (u64) [default: from config, or 42]
  --scale-factor <FACTOR>  Multiplier for all dataset num_rows
  -q, --quiet              Suppress progress output
  --list-generators        Print all generator types and their parameters, then exit
```

### Config file format

```yaml
seed: 42            # Global random seed; default 42; overridable via --seed
scale_factor: 1.0   # Multiplier for all dataset num_rows; default 1.0; overridable via --scale-factor

datasets:
  - name: <dataset_name>         # Referenced by foreign_key generators
    output: <output_directory>    # Where to write output Parquet file(s)
    num_rows: <integer>           # Row count before scale_factor
    seed: <integer>               # Per-dataset seed override (optional)
    partition:                    # Hive-style date partitioning (optional)
      column: <col_name>          # Column name for partition key
      start: "YYYY-MM-DD"         # First partition date
      days: <integer>             # Number of daily partitions
    entity:                       # Entity pool configuration (optional)
      pool_ratio: <float>         # pool_size = num_rows × pool_ratio
      columns:                    # Entity-level columns (stable per entity)
        - name: <col>
          generator: { ... }
    columns:                      # Row-level columns
      - name: <col>
        generator: { ... }
```

### Generator types

**Identifiers:**

| Type | Parameters | Description |
|------|------------|-------------|
| `sequential_id` | — | Auto-incrementing integer starting at 1 |
| `uuid` | — | Deterministic UUID v4 |
| `foreign_key` | `dataset: <name>` | Random integer in `[1, referenced_dataset_row_count]`; referenced dataset must appear earlier in config |

**Strings:**

| Type | Parameters | Description |
|------|------------|-------------|
| `constant` | `value: <string>` | Fixed value for every row |
| `one_of` | `values: [...]` | Uniform random selection from list |
| `weighted_choice` | `values: {v: weight, ...}` | Weighted random selection; weights normalized |
| `string_pattern` | `template: <string>` | Template with `{sequential_id}`, `{uuid}`, `{uniform_int:MIN-MAX}`, `{one_of:a,b,c}` placeholders |

**Numbers:**

| Type | Parameters | Description |
|------|------------|-------------|
| `uniform_int` | `min`, `max` | Uniform integer in `[min, max)` |
| `uniform_float` | `min`, `max` | Uniform float in `[min, max)` |
| `log_normal` | `median`, `sigma`, `max` | Log-normal distribution; `max` caps output |
| `geometric` | `p`, `min` (default `1`) | Geometric distribution; `min` defaults to `1` |

**Dates and timestamps:**

| Type | Parameters | Description |
|------|------------|-------------|
| `date` | `start`, `end` (YYYY-MM-DD) | Random date in `[start, end)`; output as `YYYY-MM-DD` string |
| `timestamp` | `start`, `end` (YYYY-MM-DDTHH:MM:SS) | Random timestamp in range; output as ISO 8601 string |

**Boolean and nullable:**

| Type | Parameters | Description |
|------|------------|-------------|
| `bool` | `prob: <float>` | Boolean; probability `prob` of `true` |
| `optional` | `prob: <float>`, `inner: <generator>` | Produces `null` with probability `1 - prob`; otherwise delegates to `inner` |

### Output format

Each dataset writes Parquet files:

- **Without partitioning**: `<output>/data.parquet` — single file.
- **With partitioning**: Hive-style directory layout: `<output>/<col>=<value>/data.parquet` for each partition value.

### Scale factor

The effective row count for each dataset is `floor(num_rows × scale_factor)`. When `scale_factor < 1`, `foreign_key` generators automatically adjust their upper bound to match the scaled dimension table size, preserving referential integrity.

## Semantics

### Determinism

Datagen uses **ChaCha8** PRNG seeded from a `u64` value. The seed hierarchy:

1. `--seed` CLI flag (overrides all)
2. Per-dataset `seed:` in config
3. Global `seed:` in config (default: `42`)

Given the same seed, config, and `smelt-datagen` binary version, output is bit-for-bit identical. Output is **not** guaranteed stable across `smelt-datagen` version upgrades — the generator implementations may change.

### Dataset ordering and foreign keys

Datasets are processed in the order they appear in the config. A `foreign_key` generator references a dataset by name; that dataset must be listed earlier so its row count is known when generating the dependent dataset. Forward references are a configuration error.

### Entity pools

When `entity:` is configured, datagen pre-generates a pool of `floor(num_rows × pool_ratio)` entities. Each row randomly selects one entity from the pool. Entity attributes (columns declared under `entity.columns`) are generated once per entity and reused across all rows that select it. Row-level columns (under the dataset's `columns`) are generated independently for each row regardless of entity.

### Partitioning

When `partition:` is configured:
- `num_rows` total rows are distributed evenly across `days` partitions.
- Each partition writes to `<output>/<column>=<YYYY-MM-DD>/data.parquet`.
- The partition column is a row-level column of type `DATE` string.
- Sequential IDs and foreign keys are assigned globally across all partitions (not restarted per partition).

## Design

**Parquet output, not CSV.** Parquet preserves column types exactly (no string-coercion ambiguity), is natively readable by DuckDB and Spark, and compresses large datasets efficiently. The trade-off is that Parquet files are binary and not human-readable — for small fixtures, CSV seeds or inline YAML test data are more appropriate. CSV was rejected because its type information is lossy: a `DATE` column round-trips through CSV as a `VARCHAR` unless the reader applies inference rules that may disagree with smelt's. Generating large CSV fixtures that feed type-sensitive downstream models is therefore fragile.

**ChaCha8 RNG, not system random.** Platform-independent determinism is essential for reproducible CI. ChaCha8 is fast and produces high-quality randomness for data generation without the platform variance of `rand::thread_rng()`. The per-dataset seed override allows individual datasets to be regenerated in isolation while keeping others stable. `rand::thread_rng()` was rejected because it produces different sequences on different platforms and OS kernel versions, making CI non-deterministic across Mac/Linux or across kernel upgrades.

**Entity pools for realistic cardinality.** Real datasets have a smaller number of distinct entities (users, devices, customers) than rows (events, orders, sessions). The `pool_ratio` mechanism models this: a 10M-row event dataset might have 2M unique visitors (`pool_ratio: 0.2`), each with consistent attributes across their sessions.

**Foreign keys by sequential ID convention.** The `foreign_key` generator assumes dimension tables use `sequential_id` (1, 2, ..., N). This convention simplifies the implementation: the generator only needs to know the dimension table's row count to produce valid references. Custom ID types (UUIDs, strings) cannot be foreign-key targets today. Supporting arbitrary ID columns was rejected for v1 because it requires the generator to read the already-generated dimension data at generation time, introducing ordering dependencies between datasets; the sequential-ID convention eliminates that dependency entirely.

**`geometric` defaults to `min: 1`.** The raw geometric distribution starts at 0, but count data (quantities, page views, purchase counts) is almost always positive. Defaulting to `min: 1` prevents callers from accidentally generating zero-count rows. Users who need the zero case must explicitly set `min: 0`.

## Constraints & Invariants

1. **Foreign key datasets must precede referencing datasets.** Processing order is config order; forward references are an error at generation time.
2. **Determinism per seed.** Same seed + same config + same binary version → same output. Cross-version stability is not guaranteed.
3. **Scale factor applies to all datasets.** Per-dataset row counts cannot override the global `scale_factor` when set via CLI flag.
4. **Entity pool size is a fraction of row count.** `pool_size = floor(num_rows × pool_ratio)` — the pool is not an absolute size.
5. **Partition column values span exactly `days` days starting at `start`.** No skipping, no gaps, no irregular intervals.

## Known Divergences / Open Questions

- **`string_pattern` determinism with `{uuid}`.** UUID generation inside `string_pattern` uses the same RNG stream as other generators; the exact UUID output across smelt-datagen versions is not guaranteed stable.
- **`log_normal` parameter semantics.** The `median` and `sigma` parameters are documented at the distribution level but the exact formula (e.g., whether sigma is the log-space standard deviation) is not stated here. See the generator source for the exact implementation.
- **No `smelt-datagen` integration in `smelt build`.** `smelt-datagen` is a standalone CLI; it does not run as part of `smelt build`. The user must run it manually or as a separate CI step. Integration is not planned but is a known workflow gap.
- **Output schema is implicit.** The output Parquet schema is inferred from the generator types, not declared explicitly. Parquet column types for each generator type are documented in the user guide but not enforced programmatically.

## References

- **Code**:
  - `crates/smelt-datagen/src/main.rs` — CLI entry point, seed resolution, scale factor
  - `crates/smelt-datagen/src/generic.rs` — `Gen<T>` trait, `ChaCha8Rng` usage, dataset generation
  - `crates/smelt-datagen/src/generators.rs` — generator type implementations
  - `crates/smelt-datagen/src/parquet.rs` — Parquet output, partitioning layout
  - `crates/smelt-datagen/src/session.rs` — entity pool implementation
- **User docs**:
  - `docs-site/docs/guide/datagen.md`
- **Related specs**:
  - `seeds.md` — CSV-based approach for small reference data
  - `sources.md` — how to wire datagen output as smelt sources
  - `testing.md` — inline YAML test data for unit tests
