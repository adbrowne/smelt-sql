---
feature: datagen
status: experimental
last_reviewed: 2026-05-17
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

**Composite / structured:**

| Type | Parameters | Description |
|------|------------|-------------|
| `json_object` | `fields: { <key>: <generator>, ... }` | Emits a JSON-encoded object as a single `Utf8` column. Each inner sub-generator produces one field; the resulting value is `{"<key1>": <value1>, ...}` |

Example:

```yaml
- name: payload
  generator:
    type: json_object
    fields:
      event_type:
        type: one_of
        values: [page_view, click, purchase]
      page_url:
        type: string_pattern
        template: "https://example.com/p/{uniform_int:1-1000}"
      session_seconds:
        type: uniform_int
        min: 0
        max: 3600
      logged_in:
        type: bool
        prob: 0.7
      referrer:
        type: optional
        prob: 0.6
        inner:
          type: one_of
          values: [google, direct, email]
```

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

### `json_object` encoding

The `json_object` generator emits one `Utf8` column whose every value is a syntactically valid JSON object:

- The output column's Arrow type is `Utf8`, never a Parquet `Struct`. The generator owns serialization; downstream models parse the column with JSON functions (`json_extract`, `read_json_auto`, etc.).
- **Field iteration order**. The `fields:` mapping is parsed into an order-preserving map; the emitted JSON object lists fields in YAML declaration order. Two runs with identical seed and config produce byte-identical JSON strings, including key order.
- **RNG consumption order**. Inner sub-generators are invoked in the same order, so reordering fields changes the seed-dependent values they observe. Reordering fields in the YAML is therefore a content change, not a no-op.
- **Per-type encoding rules**. Each `GenericValue` produced by an inner sub-generator is encoded as follows:
  - `Int`, `Float` — unquoted JSON number. `Float` uses Rust's standard `f64` `Display` (no scientific notation unless required); `NaN` and `Inf` are unrepresentable and must not be produced by sub-generators (callers' responsibility — `log_normal` etc. clamp to finite ranges).
  - `Bool` — unquoted JSON `true` / `false`.
  - `Str` — JSON-escaped string in double quotes. The escaper handles `"`, `\`, `\b`, `\f`, `\n`, `\r`, `\t`, and any control character `< 0x20` via the `\uXXXX` form. Non-ASCII characters are passed through unescaped (the output is UTF-8).
  - `Null` (from an `optional` sub-generator that fired) — unquoted JSON `null`. **The field is always present** in the object; the value is `null`. A `json_object` does not omit fields.
- **Nesting**. An inner sub-generator may itself be `json_object`; the nested object is serialised as an embedded JSON object value (no double-encoding). Nesting depth is not formally bounded but is in practice limited by recursion depth in `apply_spec`.
- **Entity vs row scope**. A `json_object` declared under `entity.columns` is generated once per entity (sticky JSON payload across that entity's rows); under the dataset's `columns` it is generated independently per row.
- **`fields:` must be non-empty.** An empty `fields:` map is a configuration error (caught at deserialization). The minimal valid `json_object` has one field.

The generator participates in the same determinism guarantee as the rest of `smelt-datagen` (§Determinism above): same seed + same config + same binary version → byte-identical Parquet output.

## Design

**Parquet output, not CSV.** Parquet preserves column types exactly (no string-coercion ambiguity), is natively readable by DuckDB and Spark, and compresses large datasets efficiently. The trade-off is that Parquet files are binary and not human-readable — for small fixtures, CSV seeds or inline YAML test data are more appropriate. CSV was rejected because its type information is lossy: a `DATE` column round-trips through CSV as a `VARCHAR` unless the reader applies inference rules that may disagree with smelt's. Generating large CSV fixtures that feed type-sensitive downstream models is therefore fragile.

**ChaCha8 RNG, not system random.** Platform-independent determinism is essential for reproducible CI. ChaCha8 is fast and produces high-quality randomness for data generation without the platform variance of `rand::thread_rng()`. The per-dataset seed override allows individual datasets to be regenerated in isolation while keeping others stable. `rand::thread_rng()` was rejected because it produces different sequences on different platforms and OS kernel versions, making CI non-deterministic across Mac/Linux or across kernel upgrades.

**Entity pools for realistic cardinality.** Real datasets have a smaller number of distinct entities (users, devices, customers) than rows (events, orders, sessions). The `pool_ratio` mechanism models this: a 10M-row event dataset might have 2M unique visitors (`pool_ratio: 0.2`), each with consistent attributes across their sessions.

**Foreign keys by sequential ID convention.** The `foreign_key` generator assumes dimension tables use `sequential_id` (1, 2, ..., N). This convention simplifies the implementation: the generator only needs to know the dimension table's row count to produce valid references. Custom ID types (UUIDs, strings) cannot be foreign-key targets today. Supporting arbitrary ID columns was rejected for v1 because it requires the generator to read the already-generated dimension data at generation time, introducing ordering dependencies between datasets; the sequential-ID convention eliminates that dependency entirely.

**`geometric` defaults to `min: 1`.** The raw geometric distribution starts at 0, but count data (quantities, page views, purchase counts) is almost always positive. Defaulting to `min: 1` prevents callers from accidentally generating zero-count rows. Users who need the zero case must explicitly set `min: 0`.

**`json_object` emits a `Utf8` column, not a Parquet `Struct`.** Real production event pipelines (Snowplow, Cloudflare logs, Segment, internal stream platforms) ship event payloads as JSON-encoded strings in a single column, and downstream models parse them with `json_extract`/`json_value`. Generating a `Struct` column would skip the parsing step that the resulting smelt example is meant to exercise — and would push the JSON-shape concern into the Arrow schema layer, where evolving the payload across versions is mechanically harder. A `Utf8` column also lets a single dataset hold heterogeneous payload shapes (different event types with different fields) under one schema, which is the realistic case. The cost — losing per-field type checking at write time — is acceptable because the model layer recovers it via `json_extract(... AS TYPE)`. A native `Struct` variant remains available as future work (see Known Divergences).

**`json_object` fields are an ordered map, not a list of `{name, generator}` pairs.** The YAML reads as `event_type: { type: one_of, ... }` — the field name is the map key. This matches how engineers think about a payload schema (a record of named fields, not a sequence of column-configs) and is shorter than the alternative `fields: [ - name: event_type, generator: ... ]`. Iteration order is preserved via `serde_yaml`'s tagged-map deserialisation (`IndexMap`-style ordering), giving deterministic JSON output without a separate `order:` field.

**`json_object` always emits the field, even when `optional` fires `null`.** An object with absent fields would force every downstream `json_extract` callsite to handle both "field missing" and "field present, value null". Always emitting the key (with `null` for optional sub-generators) keeps the consuming SQL simpler: the existence test collapses to a `NOT NULL` check on the extracted value. Models that genuinely care about presence-vs-null can use `optional` at the outer `json_object` boundary (entire payload optional) rather than inside.

## Constraints & Invariants

1. **Foreign key datasets must precede referencing datasets.** Processing order is config order; forward references are an error at generation time.
2. **Determinism per seed.** Same seed + same config + same binary version → same output. Cross-version stability is not guaranteed.
3. **Scale factor applies to all datasets.** Per-dataset row counts cannot override the global `scale_factor` when set via CLI flag.
4. **Entity pool size is a fraction of row count.** `pool_size = floor(num_rows × pool_ratio)` — the pool is not an absolute size.
5. **Partition column values span exactly `days` days starting at `start`.** No skipping, no gaps, no irregular intervals.

## Known Divergences / Open Questions

- **`json_object` has no array / list field type.** A field whose JSON value is an array (e.g. an `items: [...]` cart payload) cannot be expressed in v1. A `json_array` companion generator is the planned extension; until then, callers needing array-valued fields must shape them as count-prefixed scalars (`item_count`, `item_0_sku`, etc.) or post-process the JSON in a model.
- **`json_object` produces a `Utf8` column, not a Parquet `Struct`.** This is the documented design choice (see §Design), but engines that prefer typed nested data (Spark, Iceberg readers) lose the schema. A native `parquet_struct` companion generator is an open question — tracked alongside the `json_array` extension.
- **`json_object` floats use Rust `f64` `Display`.** Cross-locale formatting (e.g. comma decimal separators) is not a concern because Rust's `f64` `Display` is locale-independent, but the exact textual form (e.g. `1.0` vs `1`, when an integer-valued float is produced by a non-integer sub-generator) is not pinned across smelt-datagen versions — the determinism guarantee covers a single binary version.
- **`string_pattern` determinism with `{uuid}`.** UUID generation inside `string_pattern` uses the same RNG stream as other generators; the exact UUID output across smelt-datagen versions is not guaranteed stable.
- **`log_normal` parameter semantics.** The `median` and `sigma` parameters are documented at the distribution level but the exact formula (e.g., whether sigma is the log-space standard deviation) is not stated here. See the generator source for the exact implementation.
- **No `smelt-datagen` integration in `smelt build`.** `smelt-datagen` is a standalone CLI; it does not run as part of `smelt build`. The user must run it manually or as a separate CI step. Integration is not planned but is a known workflow gap.
- **Output schema is implicit.** The output Parquet schema is inferred from the generator types, not declared explicitly. Parquet column types for each generator type are documented in the user guide but not enforced programmatically.
- **Generator files cannot produce `smelt-datagen` config files; generators-of-generators are forbidden.** The `generates: models` frontmatter directive (per `meta_language.md` §"Multi-model production") produces SQL model definitions, not `smelt-datagen` `.yml` configs. A generator file that attempts to emit another generator's source — by writing a meta-language expression whose evaluated output is itself a generator file's body, or by referencing another generator's emitted model via `smelt.<path>` — is a hard error in v1 (forbidden by `GeneratorBodyForbidsModelReflection` and the rule that literal `smelt.<path>` references inside a generator body resolve only against hand-authored models). The `smelt-datagen` toolchain and the meta-language generator surface are deliberately non-overlapping: datagen produces sample input rows in Parquet, multi-model production produces SQL model definitions. Workspaces wanting "one config describes multiple datagen runs" must script that outside the meta-language. Tracked in `docs/plans/20260509-meta-language-overall.md`.

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
