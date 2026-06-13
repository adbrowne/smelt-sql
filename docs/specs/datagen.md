---
feature: datagen
status: experimental
last_reviewed: 2026-05-18
owners: [andrew]
---

# Datagen

> **What this is.** A normative spec for `smelt-datagen` — the deterministic test-data generation tool. Covers the YAML config format, generator types, entity pools, foreign keys, partitioning, and determinism guarantee. Out of scope: small CSV reference data (see `seeds.md`); wiring generated Parquet as sources (see `sources.md`); inline unit-test data (see `testing.md`).
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

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
    linked_pools:                 # Pre-computed joint-distribution pools (optional)
      - name: <pool_name>         # Referenced by linked_choice generators
        pool_size: <integer>      # Number of pool entries (tuples) to pre-build
        seed: <integer>           # Per-pool seed override (optional)
        shapes:                   # Weighted shape templates; pool is sampled from these
          - weight: <float>
            emit: <integer>       # Pool entries produced per draw of this shape (default 1)
            sticky: [<field>, ...] # Fields drawn once per draw and repeated across emitted entries
            fields:               # Tuple shape — keys are the pool's field names
              <field>: <generator>
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
| `linked_choice` | `pool: <name>`, `field: <field_name>` | Emits one field of a tuple drawn from a dataset-level `linked_pools` entry. Multiple `linked_choice` columns in the same row that reference the same `pool` see the *same* tuple, producing correlated values across columns (e.g. matched `(device_id, user_id)` pairs with realistic co-occurrence). |

Example — `json_object`:

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

Example — `linked_choice` modelling realistic device/user co-occurrence:

```yaml
- name: events
  output: data/events
  num_rows: 1000000
  linked_pools:
    - name: device_user
      pool_size: 200000
      seed: 7
      shapes:
        - weight: 0.60                       # single-owner: 1 device → 1 user
          fields:
            device_id: { type: foreign_key, dataset: devices }
            user_id:   { type: foreign_key, dataset: users }
        - weight: 0.25                       # anonymous: device with no logged-in user
          fields:
            device_id: { type: foreign_key, dataset: devices }
            user_id:
              type: optional
              prob: 0.0
              inner: { type: foreign_key, dataset: users }
        - weight: 0.10                       # shared device: same device, 2 users
          emit: 2
          sticky: [device_id]
          fields:
            device_id: { type: foreign_key, dataset: devices }
            user_id:   { type: foreign_key, dataset: users }
        - weight: 0.05                       # multi-device user: same user, 2 devices
          emit: 2
          sticky: [user_id]
          fields:
            device_id: { type: foreign_key, dataset: devices }
            user_id:   { type: foreign_key, dataset: users }
  columns:
    - name: device_id
      generator: { type: linked_choice, pool: device_user, field: device_id }
    - name: user_id
      generator: { type: linked_choice, pool: device_user, field: user_id }
    - name: event_time
      generator:
        type: timestamp
        start: "2026-01-01T00:00:00"
        end:   "2026-03-01T00:00:00"
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

### `linked_choice` and joint-distribution pools

A `linked_pools:` entry under `DatasetConfig` declares a **pre-computed joint-distribution pool** — a list of tuples generated once before any data rows are written. `linked_choice` column generators look one field up from a per-row randomly-selected pool entry. Multiple `linked_choice` columns in the same row that reference the same `pool` see the **same** tuple, so their values are correlated.

**Pool construction.**

1. The pool is built in a dedicated RNG stream seeded from the per-pool `seed:` if present, otherwise `dataset_seed.wrapping_add(100 + linked_pool_index)` (deterministic, distinct from the entity-pool stream and from the row stream). This isolates pool generation from row generation: changing `num_rows` does not perturb pool contents.
2. Shapes are drawn until the pool has at least `pool_size` entries. Each draw:
   - Samples one shape from `shapes:` according to its `weight:` (weights are normalised; identical to `weighted_choice` semantics).
   - For each field listed in `sticky:`, draws the value **once** for this shape draw.
   - Emits `emit:` (default `1`) entries into the pool. Sticky fields share the once-drawn value across all emitted entries; non-sticky fields are redrawn independently for each emitted entry.
3. If the last shape draw overshoots `pool_size`, the surplus entries are truncated. The final pool has exactly `pool_size` entries.
4. `shapes:` must be non-empty; every shape's `weight:` must be `> 0`; every shape must declare the same `fields:` keys (the pool's tuple schema is uniform across shapes). `sticky:` fields must be a subset of `fields:`. `emit: 0` is a configuration error.

**Field generators inside shapes.** Field generators inside `shapes[].fields` are the same `GeneratorSpec` types used for column generators, with one restriction: `linked_choice` is **not** allowed inside `shapes[].fields` (pools cannot reference other pools — this avoids ordering ambiguity and circular references). `foreign_key` is allowed and resolves against the same `fk_counts` map row-level columns use.

**Row-time sampling.** Each event row, in the order columns are written, picks one pool entry index by drawing a uniform integer in `[0, pool_size)` from the row RNG stream. The *same* index is used for every `linked_choice` column in that row that references the same pool, so the row sees one whole tuple. Different pools sample independently. The pool-index draw happens exactly once per (row, pool) — not once per `linked_choice` column.

**Field iteration order inside a shape.** Like `json_object`, `shapes[].fields` is an order-preserving map. Field generators consume RNG state in declaration order. Reordering `fields:` keys is therefore a content change for the pool. The on-disk tuple representation is field-name keyed (column lookup), so reordering does not change which field a `linked_choice` column resolves — but it does change the field's value under a fixed seed.

**Arrow type per field.** A `linked_choice` column's Arrow type is the Arrow type of the referenced field's generator, evaluated against the first shape that declares the field. (All shapes must agree on field generator output types — see Constraints.) Nullability follows the same rule: nullable iff the referenced field's generator is nullable (`Optional`).

**Interaction with `entity:`.** `linked_choice` columns are row-level columns, not entity columns. They cannot appear under `entity.columns`. A row that has both an entity attribute and a `linked_choice` column sees the entity row's sticky attribute *and* a freshly drawn pool entry; the two are independent. A future spec extension may allow `linked_choice` under `entity.columns` (sticky pool entry per entity); not in v1.

**Interaction with partitioning.** Pools are built once per dataset and shared across all partitions. Each partition's per-row sampling uses its own RNG stream (per the existing `day_seeds[]` mechanism), so partitions sample independently from the shared pool. Partitions do not regenerate the pool.

**Determinism.** Same seed + same config + same binary version → byte-identical pool contents → byte-identical row-level draws → byte-identical Parquet output. The pool seed is not affected by changes to row-level columns elsewhere in the dataset.

## Design

**Parquet output, not CSV.** Parquet preserves column types exactly (no string-coercion ambiguity), is natively readable by DuckDB and Spark, and compresses large datasets efficiently. The trade-off is that Parquet files are binary and not human-readable — for small fixtures, CSV seeds or inline YAML test data are more appropriate. CSV was rejected because its type information is lossy: a `DATE` column round-trips through CSV as a `VARCHAR` unless the reader applies inference rules that may disagree with smelt's. Generating large CSV fixtures that feed type-sensitive downstream models is therefore fragile.

**ChaCha8 RNG, not system random.** Platform-independent determinism is essential for reproducible CI. ChaCha8 is fast and produces high-quality randomness for data generation without the platform variance of `rand::thread_rng()`. The per-dataset seed override allows individual datasets to be regenerated in isolation while keeping others stable. `rand::thread_rng()` was rejected because it produces different sequences on different platforms and OS kernel versions, making CI non-deterministic across Mac/Linux or across kernel upgrades.

**Entity pools for realistic cardinality.** Real datasets have a smaller number of distinct entities (users, devices, customers) than rows (events, orders, sessions). The `pool_ratio` mechanism models this: a 10M-row event dataset might have 2M unique visitors (`pool_ratio: 0.2`), each with consistent attributes across their sessions.

**Foreign keys by sequential ID convention.** The `foreign_key` generator assumes dimension tables use `sequential_id` (1, 2, ..., N). This convention simplifies the implementation: the generator only needs to know the dimension table's row count to produce valid references. Custom ID types (UUIDs, strings) cannot be foreign-key targets today. Supporting arbitrary ID columns was rejected for v1 because it requires the generator to read the already-generated dimension data at generation time, introducing ordering dependencies between datasets; the sequential-ID convention eliminates that dependency entirely.

**`geometric` defaults to `min: 1`.** The raw geometric distribution starts at 0, but count data (quantities, page views, purchase counts) is almost always positive. Defaulting to `min: 1` prevents callers from accidentally generating zero-count rows. Users who need the zero case must explicitly set `min: 0`.

**`json_object` emits a `Utf8` column, not a Parquet `Struct`.** Real production event pipelines (Snowplow, Cloudflare logs, Segment, internal stream platforms) ship event payloads as JSON-encoded strings in a single column, and downstream models parse them with `json_extract`/`json_value`. Generating a `Struct` column would skip the parsing step that the resulting smelt example is meant to exercise — and would push the JSON-shape concern into the Arrow schema layer, where evolving the payload across versions is mechanically harder. A `Utf8` column also lets a single dataset hold heterogeneous payload shapes (different event types with different fields) under one schema, which is the realistic case. The cost — losing per-field type checking at write time — is acceptable because the model layer recovers it via `json_extract(... AS TYPE)`. A native `Struct` variant remains available as future work (see Known Divergences).

**`json_object` fields are an ordered map, not a list of `{name, generator}` pairs.** The YAML reads as `event_type: { type: one_of, ... }` — the field name is the map key. This matches how engineers think about a payload schema (a record of named fields, not a sequence of column-configs) and is shorter than the alternative `fields: [ - name: event_type, generator: ... ]`. Iteration order is preserved via `serde_yaml`'s tagged-map deserialisation (`IndexMap`-style ordering), giving deterministic JSON output without a separate `order:` field.

**`json_object` always emits the field, even when `optional` fires `null`.** An object with absent fields would force every downstream `json_extract` callsite to handle both "field missing" and "field present, value null". Always emitting the key (with `null` for optional sub-generators) keeps the consuming SQL simpler: the existence test collapses to a `NOT NULL` check on the extracted value. Models that genuinely care about presence-vs-null can use `optional` at the outer `json_object` boundary (entire payload optional) rather than inside.

**`linked_choice` is a pool-and-reference, not a tuple-returning generator.** A tuple-returning generator would have to emit values into multiple columns in one call — but smelt-datagen's column model is fundamentally one `GeneratorSpec` → one column. Rather than reshape the column model, `linked_choice` follows the existing `EntityPool` precedent: a dataset-level pre-built data structure that columns reference by name. The pool is built once before any rows are written; row generation just samples an index and looks values up. This keeps `apply_spec` per-column and pure, keeps `linked_choice` composable with all other generators (partitioning, scaling, `--list-generators`), and reuses the existing rayon-parallel partition writer unchanged. The cost is two YAML concepts instead of one (`linked_pools:` definition + `linked_choice` reference); the alternative — a "multi-column emit" generator — would require row-shape changes touching `generate_row`, `rows_to_record_batch`, schema construction, and the partition path simultaneously.

**Joint distribution by weighted shape templates with `emit:` and `sticky:`, not by raw tuple lists.** A naive "pool of explicit tuples" requires the user to spell out every entry, which doesn't scale past toy datasets. Weighted *shape templates* let the user describe a co-occurrence pattern at a much higher level: "60% of pool entries look like X, 25% look like Y, 10% are shared-device shapes Z." The `emit:` knob produces N pool entries per draw, and `sticky:` says which fields are shared across those N entries. Together this models the four realistic co-occurrence cases the example pipeline needs:

  | Real-world shape | YAML expression |
  |---|---|
  | Single-owner device (1 device → 1 user) | `emit: 1`, no `sticky:` |
  | Anonymous device (device, no user) | `emit: 1`, `user_id` is `optional { prob: 0 }` |
  | Shared device (1 device, N users) | `emit: N`, `sticky: [device_id]` |
  | Multi-device user (1 user, N devices) | `emit: N`, `sticky: [user_id]` |

A raw tuple-list mode was rejected for v1 because it provides no abstraction over the user's input file — every distribution change requires re-spelling every tuple. The shape-template mode collapses each pattern to a single weighted entry. Statistical fine-tuning (controlling exact share of repeat counts) is left to future work — for the realism level needed by example pipelines and unit tests, the four-shape vocabulary is sufficient and the resulting pool's `(device_id, user_id)` co-occurrence is testable end-to-end.

**Pool RNG stream is isolated from the row RNG stream.** Pool construction uses its own seed (per-pool `seed:` if present, else `dataset_seed.wrapping_add(100 + linked_pool_index)`), distinct from both the entity-pool seed (`dataset_seed.wrapping_add(1)`) and the row seed. The `+100` base offset deliberately skips the range `[1, 99]` used by entity-related seeds today (`+1` for entity pool, `+2` for day-seed generation), leaving room for future entity-related seed additions without retroactively colliding with any linked-pool seed. Changing `num_rows` or row-level columns therefore does not perturb the pool contents — a property that matters when an example pipeline wants to vary scale (`--scale-factor`) while keeping the device/user universe identical for comparability. The cost is one more deterministic seed offset to document; the alternative — sharing the row RNG — would couple pool generation to row count, which is the wrong dependency.

**`linked_choice` is forbidden inside `shapes[].fields`.** Pools cannot reference other pools. Allowing this would introduce an ordering question (which pool is built first?) and a circular-reference failure mode. Forbidding it keeps pool construction strictly local: each pool is built from primitive generators only. If a higher-order distribution is needed in future work, it would be expressed via dataset composition (one dataset's pool drawn from another dataset's already-materialised dimension), not via nested pools.

## Constraints & Invariants

1. **Foreign key datasets must precede referencing datasets.** Processing order is config order; forward references are an error at generation time.
2. **Determinism per seed.** Same seed + same config + same binary version → same output. Cross-version stability is not guaranteed.
3. **Scale factor applies to all datasets.** Per-dataset row counts cannot override the global `scale_factor` when set via CLI flag.
4. **Entity pool size is a fraction of row count.** `pool_size = floor(num_rows × pool_ratio)` — the pool is not an absolute size.
5. **Partition column values span exactly `days` days starting at `start`.** No skipping, no gaps, no irregular intervals.
6. **`linked_pools` pool size is an absolute count.** `pool_size:` is the exact number of pool entries (rows in the pool, not draws). The pool is not scaled by `--scale-factor`. Scaling the dataset's `num_rows` keeps the same pool; the row-time uniform sampling adapts automatically.
7. **`linked_pools` shapes must agree on field names and field generator output types.** Every shape's `fields:` must declare the same set of keys; the generator under a given key must produce the same Arrow type in every shape (modulo `Optional` wrapping). `linked_choice` columns reference fields by name; a missing or type-mismatched key is a configuration error.
8. **`linked_choice` is forbidden inside `shapes[].fields`.** Pools cannot reference other pools. The configuration loader rejects this at parse time.
9. **A `linked_choice` column's `pool:` and `field:` must resolve.** Referencing an undeclared pool or an undeclared field within a pool is a configuration error.

## Known Divergences / Open Questions

- **`json_object` has no array / list field type.** A field whose JSON value is an array (e.g. an `items: [...]` cart payload) cannot be expressed in v1. A `json_array` companion generator is the planned extension; until then, callers needing array-valued fields must shape them as count-prefixed scalars (`item_count`, `item_0_sku`, etc.) or post-process the JSON in a model.
- **`json_object` produces a `Utf8` column, not a Parquet `Struct`.** This is the documented design choice (see §Design), but engines that prefer typed nested data (Spark, Iceberg readers) lose the schema. A native `parquet_struct` companion generator is an open question — tracked alongside the `json_array` extension.
- **`json_object` floats use Rust `f64` `Display`.** Cross-locale formatting (e.g. comma decimal separators) is not a concern because Rust's `f64` `Display` is locale-independent, but the exact textual form (e.g. `1.0` vs `1`, when an integer-valued float is produced by a non-integer sub-generator) is not pinned across smelt-datagen versions — the determinism guarantee covers a single binary version.
- **`string_pattern` determinism with `{uuid}`.** UUID generation inside `string_pattern` uses the same RNG stream as other generators; the exact UUID output across smelt-datagen versions is not guaranteed stable.
- **`log_normal` parameter semantics.** The `median` and `sigma` parameters are documented at the distribution level but the exact formula (e.g., whether sigma is the log-space standard deviation) is not stated here. See the generator source for the exact implementation.
- **No `smelt-datagen` integration in `smelt build`.** `smelt-datagen` is a standalone CLI; it does not run as part of `smelt build`. The user must run it manually or as a separate CI step. Integration is not planned but is a known workflow gap.
- **Output schema is implicit.** The output Parquet schema is inferred from the generator types, not declared explicitly. Parquet column types for each generator type are documented in the user guide but not enforced programmatically.
- **`linked_choice` cannot be declared under `entity.columns`.** A `linked_choice` column is row-level only — the pool entry is drawn fresh per row, not per entity. Allowing sticky-per-entity pool draws is a sensible extension (one entity always sees the same tuple from a referenced pool) but adds a second sampling rule and is deferred. Workspaces that want both behaviors today should model the per-entity case by putting the correlated fields under `entity.columns` and accepting that the entity-level draws are independent rather than joint.
- **`linked_pools` has no raw-tuple-list mode.** Pools are built from weighted shape templates only. A literal `tuples: [...]` mode (explicit pool contents) is not supported in v1. The closest workaround is a single shape with `weight: 1` and field generators that draw from a deterministic seed range; a literal-tuple mode is tracked as future work.
- **`linked_pools` does not enforce shape `emit:` upper bound.** Very large `emit:` values combined with small `pool_size` can produce a pool dominated by one shape draw (e.g. a single `emit: 100` draw filling a `pool_size: 100` pool). The configuration loader does not warn — the user's weighted distribution is taken at face value. Sanity-checking is left to the example pipeline's unit tests.
- **`linked_pools` field generator output types are not statically checked across shapes.** The spec requires every shape to declare the same field generator output types, but the v1 implementation may only check the first shape's type when building the Arrow schema for `linked_choice` columns. A shape that emits a type-divergent field will silently coerce at the Arrow builder layer (per the existing `build_column` fallbacks). Promoting this to a parse-time check is tracked as follow-up; for v1 users are expected to keep shape field generators type-uniform by hand.
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
