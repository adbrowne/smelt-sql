# Data Generation

smelt includes `smelt-datagen`, a deterministic data generation tool for creating realistic test datasets. It generates Parquet files from a YAML configuration, with support for partitioning, foreign keys, entity pools, and configurable distributions.

## When to use datagen

| Approach | Best for | Size |
|----------|----------|------|
| [Seeds](seeds.md) (CSV) | Small reference data, version-controlled test fixtures | < 1,000 rows |
| **smelt-datagen** (Parquet) | Large test datasets with realistic distributions | 1K–100M+ rows |
| Inline test data | Unit test fixtures in `.test.sql` files | < 100 rows |

Use datagen when you need datasets large enough to test performance, partitioning, or incremental models — but still want deterministic, reproducible output.

## Installation

`smelt-datagen` is included with the smelt package:

```bash
pip install smelt-sql
```

Verify the installation:

```bash
smelt-datagen --help
```

## Quick start

Create a file called `datagen.yaml`:

```yaml
seed: 42

datasets:
  - name: users
    output: data/users
    num_rows: 1000
    columns:
      - name: user_id
        generator: { type: sequential_id }
      - name: name_prefix
        generator:
          type: weighted_choice
          values:
            Mr: 0.40
            Ms: 0.35
            Dr: 0.05
            "": 0.20
      - name: country
        generator:
          type: one_of
          values: [US, UK, DE, FR, CA, AU]
      - name: signup_date
        generator: { type: date, start: "2022-01-01", end: "2024-12-31" }
```

Generate the data:

```bash
smelt-datagen --config datagen.yaml
```

This creates `data/users/data.parquet`. Inspect it with DuckDB:

```bash
duckdb -c "SELECT * FROM 'data/users/data.parquet' LIMIT 10"
```

## Configuration reference

### Top-level keys

```yaml
seed: 42              # Global random seed (default: 42)
scale_factor: 1.0     # Multiplier for all dataset num_rows

datasets:
  - ...                # List of dataset configurations
```

### Dataset keys

```yaml
- name: orders            # Dataset name (used by foreign_key references)
  output: data/orders     # Output directory
  num_rows: 1000000       # Number of rows to generate
  seed: 123               # Per-dataset seed (overrides global)
  partition: ...          # Optional: Hive-style date partitioning
  entity: ...             # Optional: entity pool for sticky attributes
  columns:                # Column definitions
    - name: order_id
      generator: { type: sequential_id }
```

## Generator types

### Identifiers

#### `sequential_id`

Auto-incrementing integer starting at 1.

```yaml
generator: { type: sequential_id }
```

#### `uuid`

Deterministic UUID v4.

```yaml
generator: { type: uuid }
```

#### `foreign_key`

Random reference to another dataset's sequential ID. The referenced dataset must appear earlier in the config.

```yaml
generator: { type: foreign_key, dataset: customers }
```

### Strings

#### `constant`

Fixed value for every row.

```yaml
generator: { type: constant, value: "active" }
```

#### `weighted_choice`

Select from options with probability weights. Weights are normalized automatically.

```yaml
generator:
  type: weighted_choice
  values:
    completed: 0.70
    shipped: 0.12
    cancelled: 0.05
    returned: 0.05
```

#### `one_of`

Uniform random selection from a list.

```yaml
generator:
  type: one_of
  values: [red, green, blue, yellow]
```

#### `string_pattern`

Template-based string generation with embedded placeholders.

```yaml
generator: { type: string_pattern, template: "user_{sequential_id}@example.com" }
```

Supported placeholders:

| Placeholder | Description | Example output |
|-------------|-------------|----------------|
| `{sequential_id}` | Row index + 1 | `42` |
| `{uuid}` | Random UUID | `a1b2c3d4-...` |
| `{uniform_int:MIN-MAX}` | Random integer in range | `4527` |
| `{one_of:a,b,c}` | Random choice from list | `b` |

```yaml
# SKU codes
generator: { type: string_pattern, template: "SKU-{uniform_int:1000-9999}" }

# Usernames
generator: { type: string_pattern, template: "{one_of:alpha,beta,gamma}-{sequential_id}" }
```

### Numbers

#### `uniform_int`

Uniform random integer in `[min, max)`.

```yaml
generator: { type: uniform_int, min: 1950, max: 2005 }
```

#### `uniform_float`

Uniform random float in `[min, max)`.

```yaml
generator: { type: uniform_float, min: 0.0, max: 1.0 }
```

#### `log_normal`

Log-normal distribution, useful for prices, counts, and durations. The `median` is the center, `sigma` controls spread, and `max` caps the output.

```yaml
generator: { type: log_normal, median: 2500, sigma: 1.2, max: 500000 }
```

#### `geometric`

Geometric distribution — "number of failures before first success". Useful for count data skewed toward zero.

```yaml
generator: { type: geometric, p: 0.5 }
```

The optional `min` parameter sets a lower bound clamp on the generated value.
**`min` defaults to `1`** so the common case ("quantity / count is never zero")
Just Works — most callers want positive counts. To opt back into the raw
distribution (which starts at zero), pass `min: 0` explicitly:

```yaml
# Defaults to min: 1 — values are always ≥ 1.
generator: { type: geometric, p: 0.5 }

# Opt in to allowing zeros.
generator: { type: geometric, p: 0.5, min: 0 }
```

Tip: run `smelt-datagen --list-generators` to see this reference (and the
parameters of every other generator) from the CLI without leaving your
terminal.

### Dates and timestamps

#### `date`

Random date within a range, output as `YYYY-MM-DD` string.

```yaml
generator: { type: date, start: "2020-01-01", end: "2024-12-31" }
```

#### `timestamp`

Random timestamp within a range, output as `YYYY-MM-DDTHH:MM:SS` string.

```yaml
generator: { type: timestamp, start: "2024-01-01T00:00:00", end: "2024-03-31T23:59:59" }
```

### Boolean and nullable

#### `bool`

Boolean with configurable probability of `true`.

```yaml
generator: { type: bool, prob: 0.15 }
```

#### `optional`

Wraps another generator, producing null with probability `1 - prob`.

```yaml
generator:
  type: optional
  prob: 0.08
  inner:
    type: one_of
    values: [Defective, Wrong Item, Changed Mind]
```

### Composite / structured

#### `json_object`

Emits a JSON-encoded object as a single `Utf8` column. Each field's value is produced by an inner sub-generator; the resulting Parquet column holds strings like `{"event_type":"page_view","logged_in":true,"referrer":null}` — one self-contained JSON object per row.

This is the right shape when you want to model a real-world event-payload column: a single `Utf8` column the downstream SQL parses with `json_extract`. The example below matches the kind of `page_events.payload` you'd see in a Snowplow-style ingestion pipeline.

```yaml
- name: payload
  generator:
    type: json_object
    fields:
      event_type:
        type: weighted_choice
        values:
          page_view: 0.60
          click: 0.30
          purchase: 0.10
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

Rules to know:

- **Fields are always present.** When `optional` fires `null`, the field is emitted as `"<key>": null` — not omitted. Downstream `json_extract(payload, '$.referrer')` returns SQL `NULL` for both cases (missing path and explicit JSON null in DuckDB), so you don't have to disambiguate them in models.
- **Field order is preserved.** The YAML field declaration order is the JSON output order. Reordering fields changes the seed-dependent values too, because sub-generators are invoked in declaration order.
- **Nesting works.** A field's generator may itself be `type: json_object`; the nested object is embedded as a JSON value, not double-encoded.
- **No arrays.** v1 does not have a `json_array` generator. For array-valued fields, either pre-shape your event schema (e.g. `item_count` + numbered columns) or post-process the JSON in a downstream model.
- **Sticky payloads.** A `json_object` placed under `entity.columns` is generated once per entity and reused — useful for per-user feature flags or settings that don't change across an entity's events.

#### `linked_choice` — correlated columns from a joint-distribution pool

Use `linked_choice` when two or more columns in the same row need to be drawn **jointly**, not independently. Typical case: `(device_id, user_id)` pairs in a web-analytics events table, where most devices have one logged-in user, some devices have multiple users (shared family devices), some users have multiple devices, and some sessions are anonymous. Drawing the two columns independently gives every (device, user) pair equal probability — losing the real co-occurrence pattern.

A `linked_pools:` block, declared at the dataset level, pre-builds a list of correlated tuples. Each row picks one tuple from the pool, and any `linked_choice` column in that row references the picked tuple's field by name:

```yaml
- name: events
  output: data/events
  num_rows: 1000000
  linked_pools:
    - name: device_user
      pool_size: 200000
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
```

Rules to know:

- **Same pool, same row → same tuple.** A row picks one pool entry index; every `linked_choice` column referencing that pool sees the *same* entry. Different pools sample independently.
- **`weight`** controls each shape's probability of being drawn next during pool construction; weights are normalised.
- **`emit:`** (default `1`) is the number of pool entries one shape draw produces. `emit: 2` with `sticky: [device_id]` models a single device shared by two users.
- **`sticky:`** lists fields that are drawn **once** per shape draw and shared across the emitted entries. Non-sticky fields are redrawn for each emitted entry.
- **`pool_size`** is an absolute entry count, not a fraction. `--scale-factor` does not scale pools — the pool stays fixed and the larger row count just samples it more times, which is usually what you want for "vary scale without changing the user/device universe."
- **`seed:`** (optional) overrides the pool's RNG seed. Omitting it lets the toolchain derive a deterministic offset from the dataset seed, so the pool is still reproducible across runs without an explicit `seed:`. Pool seeds are isolated from row seeds, so changing `num_rows` never perturbs pool contents.
- **Field type uniformity.** Every shape's `fields:` must declare the same keys, and the generator under a given key must produce the same Arrow type across shapes. A `linked_choice` column's nullability is `nullable` iff the **first shape's** field generator is wrapped in `optional` — the first shape is the schema-defining shape.
- **Forbidden inside shapes.** `linked_choice` cannot itself be a field generator inside `shapes[].fields:` — pools cannot reference other pools.
- **Row-level only.** `linked_choice` columns belong under the dataset's `columns:`, not under `entity.columns:`. Pool entries are drawn per row, not per entity.

## Partitioning

Add a `partition` block to create Hive-style date-partitioned output. Each day gets its own directory with a `data.parquet` file, and rows are distributed evenly across days.

```yaml
- name: events
  output: data/events
  num_rows: 5000000
  partition:
    column: event_date
    start: "2024-07-01"
    days: 90
  columns:
    - name: event_id
      generator: { type: sequential_id }
    - name: event_type
      generator:
        type: weighted_choice
        values:
          page_view: 0.50
          click: 0.30
          purchase: 0.20
```

Output directory structure:

```
data/events/
  event_date=2024-07-01/data.parquet
  event_date=2024-07-02/data.parquet
  ...
  event_date=2024-09-28/data.parquet
```

## Entity pools

Entity pools create a fixed set of "entities" (e.g., visitors, devices) whose attributes are consistent across all rows that reference them. This models the real-world pattern where the same user appears in multiple events with the same country, device, etc.

```yaml
- name: sessions
  output: data/sessions
  num_rows: 10000000
  entity:
    pool_ratio: 0.2    # 10M rows × 0.2 = 2M unique visitors
    columns:
      - name: visitor_id
        generator: { type: uuid }
      - name: country
        generator:
          type: weighted_choice
          values:
            US: 0.40
            UK: 0.15
            DE: 0.10
            Other: 0.35
  columns:
    - name: session_id
      generator: { type: uuid }
    - name: platform
      generator:
        type: one_of
        values: [web, ios, android]
```

Each row gets a randomly selected entity from the pool. The entity's `visitor_id` and `country` are the same every time that entity appears.

## Foreign keys

Foreign keys create referential integrity between datasets. The referenced dataset must be listed earlier in the config so its row count is known.

```yaml
datasets:
  # Dimension table (listed first)
  - name: customers
    output: data/customers
    num_rows: 100000
    columns:
      - name: customer_id
        generator: { type: sequential_id }
      - name: segment
        generator:
          type: weighted_choice
          values: { Regular: 0.50, Premium: 0.25, VIP: 0.10, New: 0.15 }

  # Fact table (references customers)
  - name: orders
    output: data/orders
    num_rows: 1000000
    columns:
      - name: order_id
        generator: { type: sequential_id }
      - name: customer_id
        generator: { type: foreign_key, dataset: customers }
```

The `foreign_key` generator produces random integers in `[1, referenced_dataset_row_count]`, matching the `sequential_id` values in the dimension table.

## Scale factors

Scale factors let you resize datasets for different environments without changing the config.

In the config file:

```yaml
scale_factor: 0.01   # Generate 1% of full dataset
```

Or override via CLI:

```bash
# CI: fast, small
smelt-datagen --config datagen.yaml --scale-factor 0.01

# Development: moderate
smelt-datagen --config datagen.yaml --scale-factor 0.1

# Load testing: full size
smelt-datagen --config datagen.yaml --scale-factor 1.0
```

The scale factor multiplies each dataset's `num_rows`. Foreign key references automatically adjust to match the scaled dimension table sizes.

## Using datagen with smelt sources

A typical workflow: generate Parquet data with datagen, then configure smelt to read it as a source.

**1. Generate the data:**

```bash
smelt-datagen --config datagen.yaml
```

**2. Declare per-entity source YAMLs** under your `paths:` directory (e.g. `models/sources/raw/`):

`models/sources/raw/customers.yml`:
```yaml
path: data/customers
```

`models/sources/raw/orders.yml`:
```yaml
path: data/orders
```

**3. Write models** that reference the sources:

```sql
-- models/marts/customer_orders.sql
SELECT
    c.customer_id,
    c.segment,
    COUNT(*) as order_count
FROM smelt.sources.raw.customers c
JOIN smelt.sources.raw.orders o ON c.customer_id = o.customer_id
GROUP BY c.customer_id, c.segment
```

**4. Build:**

```bash
smelt build
```

## CLI reference

```
smelt-datagen [OPTIONS]

Options:
  --config <PATH>          YAML config file
  -o, --output <PATH>      Output directory [default: output]
  -s, --seed <SEED>        Random seed [default: 42]
  --scale-factor <FACTOR>  Scale multiplier for dataset sizes
  -q, --quiet              Suppress progress output
  -h, --help               Print help
```

## Example: retail analytics

The `examples/retail_analytics/datagen.yaml` in the smelt repository defines a complete star schema with:

- **Dimension tables**: `customers` (100K), `products` (10K), `stores` (500)
- **Fact tables**: `orders` (1M), `order_items` (3M), `web_events` (5M)

Fact tables use `foreign_key` to reference dimensions, and are date-partitioned across 90 days. Run it at 1% scale for quick iteration:

```bash
smelt-datagen --config examples/retail_analytics/datagen.yaml --scale-factor 0.01
```

## Further reading

- [Seeds](seeds.md) for small CSV-based test data
- [Sources](sources.md) for configuring external data references
- [Testing](testing.md) for inline test data in `.test.sql` files
