---
feature: seeds
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Seeds

> **Scope.** Normative spec for CSV seed loading: filesystem layout, addressing, schema declaration, type inference, materialization, cross-backend loading, and the LSP tooling that ties them together. Sources (externally-managed tables) share the same YAML schema-declaration shape and are referenced here, but their full surface lives in `sources.md` (future).

## Surface

### One-line summary

A `.csv` file under a scanned project directory is a **seed**: smelt parses it, infers (or reads from a sibling `.yml`) the column schema, and loads it into the active backend on `smelt seed` / `smelt build`. A `.yml` file with no sibling CSV is a **source**: smelt does not load it, only declares its schema. Both kinds are addressed by their workspace path under the universal `smelt.<path>` scheme.

### Filesystem layout

Smelt scans every directory listed in `smelt.yml::paths` (default `["models"]`) for project files. Inside a scanned directory:

| File found | Kind | What smelt does |
|---|---|---|
| `<dir>/<stem>.csv` (no sibling YAML) | seed (schema inferred) | Parse, infer types, load on `smelt seed` |
| `<dir>/<stem>.csv` + sibling `<dir>/<stem>.yml` | seed (schema pinned) | Parse, validate against YAML, load on `smelt seed` |
| `<dir>/<stem>.yml` (no sibling CSV) | source | Declare schema only; no load |

Subdirectories are scanned recursively. There is no `seed_paths` config and no special `seeds/` directory — by convention `models/seeds/` or `models/data/` are common places, but any path under a scanned root works.

### Addressing and database mapping

Both seeds and sources are addressed by their workspace path under universal `smelt.<path>` (`architecture.md` §"Resolution"). The scan-root prefix is stripped; the remaining path components form the address:

| File on disk (with `paths: ["models"]`) | Address | DB location (default) |
|---|---|---|
| `models/raw_orders.csv` | `smelt.raw_orders` | `<target_schema>.raw_orders` |
| `models/data/raw/users.csv` | `smelt.data.raw.users` | `<target_schema>.data_raw_users` |
| `models/payments/seeds/lookup/regions.csv` | `smelt.payments.seeds.lookup.regions` | `<target_schema>.payments_seeds_lookup_regions` |
| `models/external/api/orders.yml` (no CSV) | `smelt.external.api.orders` | (externally managed; not materialised by smelt) |

Address path components are joined with `_` to form the table name; the schema is always the active target's `schema:` from `smelt.yml`. There is no per-subdirectory schema mapping.

When `paths:` lists multiple roots (e.g., `paths: ["models", "fixtures"]`), the scan-root prefix is stripped from each independently, producing addresses in a single shared namespace. Two files that resolve to the same address — e.g., `models/users.csv` and `fixtures/users.csv` — are a hard workspace-load error.

A future configurable mapping (`generate_schema_name` / `generate_alias_name` analogue) is out of scope here and tracked in Known Divergences.

### Sidecar / source YAML shape

The same YAML shape declares the schema for both a seed sidecar and a standalone source declaration:

```yaml
description: User dimension data, refreshed weekly from the CRM export.
materialization: table
columns:
  - name: user_id
    type: INTEGER
    nullable: false
    description: Surrogate key.
  - name: user_name
    type: VARCHAR
  - name: signup_date
    type: DATE
  - name: lifetime_value
    type: DECIMAL(10, 2)
```

| Key | Required | Default | Meaning |
|-----|----------|---------|---------|
| `description` | no | absent | Free-text description of the table. Surfaced in LSP hover and (future) docs. |
| `materialization` | no (seeds only) | `table` | `table` (default) or `ephemeral`. **Must be absent on a standalone source YAML** — sources are externally managed. |
| `columns` | no | absent (full inference) | List of column declarations. When present, it is the contract: every CSV column must appear, names must match. |
| `columns[].name` | yes | — | Column name as it appears in the CSV header. Match is by name. |
| `columns[].type` | yes | — | Smelt `DataType` (`types.md`). Recognised: `BOOLEAN`, `INTEGER`, `DOUBLE`, `DECIMAL(p,s)`, `DATE`, `TIMESTAMP`, `VARCHAR`. |
| `columns[].nullable` | no | `true` | Whether the column may contain NULL. `false` triggers a hard error if any row contains a NULL in that column. |
| `columns[].description` | no | absent | Free-text description, surfaced in LSP hover. |

A YAML with `description:` only and no `columns:` is valid for a seed (description on top of full inference); it's invalid for a source (a source must declare its schema).

### Materialization values

| Value | Meaning |
|---|---|
| `table` (default for seeds) | A CREATE TABLE in the target schema; `smelt seed` loads it via the backend's Arrow ingest path. |
| `ephemeral` | No table is created; the seed is spliced into using-side SQL as a `VALUES (...)` literal at compile time. |

`view` and `materialized_view` are not currently supported for seeds and produce a hard error at load time. (Possible future addition; tracked in Known Divergences.)

### CSV format the loader accepts

The loader is strict (no per-seed override surface in v1):

- Comma delimiter.
- Double-quote quoting; embedded `"` is escaped as `""`.
- The first row is the header. Header names map directly to the column names used by `columns[].name` and SQL.
- Empty cell → `NULL` in every column type, including `VARCHAR`. There is no way to express a literal empty string in a CSV — use `COALESCE(col, '')` in a downstream model if needed.
- UTF-8 encoded; a UTF-8 BOM on the first line is consumed silently.
- LF or CRLF line endings; mixed within a file is accepted.

CSVs that do not match this format produce a hard error with file/line/column pointer. There is no auto-detection.

### Type inference

When no `columns:` is declared, smelt infers each column's type from the CSV data. Two phases consume the same inference rules:

- **Compile time** (LSP, `smelt table`, type-checking downstream models): samples the **first 100 data rows**.
- **Runtime** (`smelt seed`, `smelt build`): reads the **whole file** and infers from every row. Runtime types may be wider than compile-time types when the first 100 rows happen to fit a narrower type.

Both phases apply the same precedence:

```
BOOLEAN → DATE → TIMESTAMP → INTEGER → DECIMAL → DOUBLE → VARCHAR
```

A column matches a type when **every** non-empty sample value parses as that type. Rules per type:

- **BOOLEAN** — every value is `true` or `false` (case-insensitive).
- **DATE** — every value matches `YYYY-MM-DD` with year ∈ [1000, 9999], month ∈ [1, 12], day ∈ [1, 31]. Calendar correctness is not validated (`Feb 30` passes shape).
- **TIMESTAMP** — every value matches `YYYY-MM-DD HH:MM:SS` with optional fractional-seconds tail (`.123…`). The space separator is required; `T`-separated ISO-8601 falls back to VARCHAR. Time-zone suffixes (`Z`, `+00`, `-05`, `Australia/Sydney`) fall back to VARCHAR — `TIMESTAMP WITH TIME ZONE` is never inferred.
- **INTEGER** — every value parses as a 64-bit signed integer.
- **DECIMAL(p,s)** — every value is a fixed-precision decimal literal (digits, optional leading `-`, optional one `.`, no scientific notation). Scale `s` = max fractional digits in the sample; precision `p` = max integer digits + `s`. The cap is `DECIMAL(18, 4)`: a column whose inferred `(p, s)` would exceed `p > 18` or `s > 4` does **not** infer as DECIMAL — it falls through to DOUBLE. Pure-integer columns are caught by INTEGER first; this rule fires only on columns containing at least one value with a `.`.
- **DOUBLE** — every value parses as `f64`. This is a fall-through after DECIMAL: a column too wide for the DECIMAL cap, or one containing scientific notation (`1.5e10`), lands here.
- **VARCHAR** — fallback when no other type matches.

A column whose every value is empty (NULL) infers as VARCHAR.

### CLI surface

`smelt seed` (and the seed phase of `smelt build`) operates on every discovered seed in deterministic order (sorted by full address path). For each seed:

1. `CREATE SCHEMA IF NOT EXISTS <target_schema>`.
2. `DROP TABLE IF EXISTS` / `DROP VIEW IF EXISTS` against the target name.
3. Parse the CSV; validate against the sidecar YAML if present; convert to Arrow `RecordBatch`es.
4. `Backend::load_table(name, schema, batches)` — backend-specific ingest.

Selectors:

- `smelt seed --select <smelt-path>` — load only the named seed (e.g., `--select data.raw.users`).
- `smelt seed --select <leaf>` — match by leaf name when unambiguous.

Sources are never affected by `smelt seed`; `--select` against a source path is a hard error ("not a seed").

### Backend trait surface

```rust
trait Backend {
    async fn load_table(
        &self,
        schema: &str,
        name: &str,
        arrow_schema: &arrow::datatypes::Schema,
        batches: Vec<arrow::record_batch::RecordBatch>,
    ) -> Result<()>;
    // … other methods
}
```

Implementations:
- **DuckDB**: `Appender` API over Arrow C-data interface.
- **Spark**: `SparkSession.createDataFrame(arrow_batches).write.saveAsTable("<schema>.<name>")`.

### LSP integration

- **Diagnostic on missing sidecar YAML**: a CSV without a sibling `.yml` emits a workspace warning ("Seed schema is inferred and may drift if the CSV changes — pin it"). Severity: warning, not error. Resolved when a sidecar is added.
- **Code action: "Pin schema to sidecar YAML"**: runs the inferencer, writes the result to a sibling `.yml` next to the CSV. After running, the warning above is resolved.
- **Code action: "Re-pin schema from CSV"** (follow-up): when a sidecar exists but its column set differs from the CSV's, re-run the inferencer and overwrite. (Spec'd here, deferred in implementation; tracked in Known Divergences.)
- **Hover**: column descriptions from the sidecar/source YAML appear on hover over a column name in a model that references the seed.
- **Goto-definition**: `smelt.<path>` resolves to the CSV file (for seeds) or the YAML file (for sources).

## Semantics

1. **Filesystem path is identity.** A seed's address (`smelt.<path>`) and default DB location are derived purely from its location under a scanned path. A rename or move changes both. There is no per-seed metadata that overrides the address.

2. **Resolver tiebreaker.** A `.yml` file's kind is determined by sibling files: if a `.csv` with the same stem exists in the same directory, the YAML is a **sidecar** (binds to the seed); otherwise it is a **source** declaration. A `.yml` and a `.csv` with the same stem cannot collide — the sidecar relationship is unambiguous. (See `architecture.md` §"Resolution" for the broader within-directory uniqueness rule.)

3. **Schema-set agreement is mandatory when pinned.** When a sidecar declares `columns:`, the column set must match the CSV header exactly — same names, in any order, with no extras on either side. Mismatch → hard error at load time and a diagnostic at compile time. There is no "load only declared columns" or "infer the rest" mode.

4. **Type-coercion failures are hard.** Any value that does not parse as its declared/inferred type aborts the load with a file/row/column pointer. Smelt does not silently substitute NULL.

5. **`nullable: false` is a load-time check.** A NULL row value in a non-nullable column is a hard error. Compile-time type-checking treats the column as non-NULL when reasoning about downstream models.

6. **Idempotence.** Re-running `smelt seed` brings the database to the same state for the same `(CSV, YAML)` inputs. Re-loading replaces the table; existing rows are not appended.

7. **Compile-time and runtime inference agree on every recognised type.** Where they differ, runtime is the wider view (it sees every row, not just the first 100). When the two disagree on a recognised type, smelt emits a diagnostic at compile time so the user can pin the schema to the runtime-correct type.

8. **Empty cell is always NULL.** This rule is uniform across all column types, including `VARCHAR`. Users who need a literal empty string materialise it via `COALESCE(col, '')` in a downstream model.

9. **`materialization: ephemeral`** desugars at compile time. When a model references an ephemeral seed, the seed body is emitted as `VALUES (…)` (with explicit per-column `CAST` to preserve type fidelity) and the reference is rewritten to a CTE. No table is created; `smelt seed` does nothing for ephemeral seeds. Cross-backend: the printer is responsible for any per-dialect adjustments to the `VALUES` form.

10. **Source declarations have no load step.** A standalone YAML is metadata only; smelt does not validate that the table exists in the database. A reference to a non-existent source surfaces only at execution time as a backend error.

11. **Discovery order is deterministic.** Seeds are loaded sorted by full address path (`smelt.<path>`). This makes runs reproducible and CI-friendly.

## Design

**Two concepts, one config shape.** Seeds and sources have different lifecycles — smelt loads a seed; an external pipeline owns a source — and that distinction is real for users. Collapsing them into a single concept ("input"; "data") muddied the lifecycle question and was rejected. But every other concern is shared: column declarations, types, descriptions, future tests, compile-time hover, goto-definition. So we keep two kinds, share the YAML grammar, and share the implementation that consumes it. The kind is determined by the presence of a sibling CSV — a structural rule, not a configuration toggle.

**Per-entity YAML, not aggregate `sources.yml`.** The aggregate file violates the universal-addressing rule: every project entity lives at its addressed path, but `sources.yml` at the project root declares entities at arbitrary `sources.<schema>.<name>` paths. Splitting into per-entity YAMLs at the entity's path makes addressing literal — `data/raw/users.yml` *is* `smelt.data.raw.users`. The cost is many small files; the benefit is one rule, not two. Aggregate `sources.yml` is removed in this revision (no compat shim — see Known Divergences).

**Smelt owns CSV parsing and inference.** Earlier the loader called DuckDB's `read_csv_auto`, and a separate compile-time inferencer ran in `smelt-core`. The two could disagree (the historical TB-2 class), the runtime path was DuckDB-only, and Spark seeds had to be loaded through a side channel. Owning the parser collapses both inferencers into one, makes seeds backend-portable, and removes the entire "two views must agree" surface area. The implementation uses the `csv` crate for tokenisation/quoting (battle-tested, small, no opinionated inference of its own) and a smelt-owned inferencer that produces Arrow `RecordBatch`es. Backend-side ingest is a uniform Arrow API. The earlier DuckDB-driven loader is removed.

**Strict CSV defaults, no per-seed override surface in v1.** dbt and sqlmesh expose delimiter / quote / NULL-marker / header config per-seed. We deliberately don't, because (a) the spec stays small, (b) every override forces decisions about how it interacts with the inferencer, and (c) projects with non-standard CSVs can convert them at the source. When concrete need emerges, overrides land in the sidecar YAML — the file is already there.

**Empty cell is always NULL.** The DuckDB-inspired alternative ("empty is NULL for non-text, empty-string for VARCHAR") matches a user's mental model coming from DuckDB's `read_csv_auto`, but introduces a type-dependent rule that surprises users coming from anywhere else and requires the loader to track per-column type while parsing. Uniform "empty = NULL" is one rule, easy to explain, easy to implement, and trivially worked around with `COALESCE` downstream. The cost is that a literal empty string in a CSV cell cannot survive into a `VARCHAR` column; we judge that a small enough loss to pay for the simpler rule.

**`DECIMAL(p,s)` in the inferred type set.** The previous spec deliberately omitted DECIMAL because the runtime inferencer (DuckDB's `read_csv_auto`) sometimes emitted it and sometimes emitted DOUBLE depending on value bounds, and the compile-time path could not predict which. With smelt owning inference, the rule is deterministic: any column with a fractional component that fits within `DECIMAL(18, 4)` and is not in scientific notation infers as DECIMAL; otherwise DOUBLE. Pure-integer columns are caught earlier as INTEGER. The cap exists so wide-fixed-point values don't accidentally pin a 38-digit DECIMAL from a 100-row sample.

**No `TIMESTAMP WITH TIME ZONE` inference.** A column with mixed-zone timestamps cannot be inferred safely from text alone — the timezone of `2025-01-10 08:00:00` is ambiguous. Recognising `Z` / `+00` suffixes and emitting `TIMESTAMP WITH TIME ZONE` is technically possible but invites silent bugs when rows mix zones or the producer changes format. Falling back to VARCHAR forces an explicit `CAST` in a downstream model, which is exactly the right place for the user to declare their timezone intent. This trades convenience for correctness.

**Path-joined-with-underscore for DB names.** A subdirectory-becomes-schema rule (today's behaviour) collides with the universal-addressing scheme: `smelt.data.raw.users` is a four-component address that has to materialise into a two-component database name (`<schema>.<table>`). The clean rule — schema = target schema, table = path joined with `_` — keeps the spec uniform across every depth and avoids inventing a special case for "first path component is a schema". The cost is uglier table names for deeply-nested paths; the user can rename a table via the future `generate_alias_name` analogue when one exists.

**Address by path; configurable DB mapping deferred.** Today's rule (path-joined-with-underscore) is the floor. Allowing users to override the mapping (à la dbt's `generate_schema_name` / `generate_alias_name` macros, or sqlmesh's table-aliasing) is a real feature but is owned by a future spec, not this one. The seeds spec defines the *default*; a configurable mapping rule overrides the default uniformly across kinds.

**LSP-driven schema pinning.** A pinned schema is the safer state — drift is impossible because every column comes from the YAML — but typing column declarations by hand is friction. The "Pin schema to sidecar YAML" code action removes that friction: smelt infers, the user one-click commits the result. The "no sidecar" warning then nudges users to take the safer path. This is the schema equivalent of `cargo fmt --emit files`: the right answer is mechanical, so smelt offers to write it.

**Rejected alternatives.**

- *Auto-detect delimiter/quote like DuckDB.* Surface that magic only matters when a user has a non-standard CSV; we'd rather have them convert it explicitly or override in the (future) sidecar config.
- *Aggregate `sources.yml` retained as legacy.* Pre-1.0 + the workspace's "no backward compatibility" policy lets us cut cleanly. A `smelt migrate` follow-up command can mechanise the rewrite; it is out of scope here.
- *Tests on seed columns now.* The architecture spec defers `smelt.test` semantics; reserving a `tests:` key now makes promises we cannot keep. Tests land in this YAML when the tests spec exists.
- *`view` and `materialized_view` materialization for seeds.* A view backed by `VALUES` is technically possible but offers little over `ephemeral` (inline) or `table` (real). Out of scope until a concrete need emerges.

## Constraints & Invariants

1. A `.csv` file is a seed; a `.yml` file with no sibling `.csv` is a source. The two are disjoint kinds; resolution is structural, not configurational.
2. The compile-time and runtime CSV inferencers are the same code path with different sample sizes. They cannot diverge by construction.
3. The CSV parser is strict: comma, double-quote, mandatory header, UTF-8, empty cell = NULL. No auto-detection.
4. The inferred type set is exactly `{BOOLEAN, INTEGER, DECIMAL(p,s), DOUBLE, DATE, TIMESTAMP, VARCHAR}`. `TIMESTAMP WITH TIME ZONE`, `TIME`, `INTERVAL`, and complex types are never inferred from CSV.
5. When a sidecar YAML declares `columns:`, the CSV header column set must match exactly (by name); mismatch is a hard error.
6. Type-coercion failures during load (parse failure, NULL in `nullable: false` column) are hard errors with file/row/column pointers — never silent NULLs.
7. Seeds materialised as `table` are loaded via the backend's `Backend::load_table(...)` Arrow ingest path. There is no DuckDB-specific or Spark-specific shortcut.
8. Seeds with `materialization: ephemeral` are never loaded into the database; they are spliced as `VALUES (…)` at compile time.
9. Sources are never loaded by `smelt seed` or `smelt build`. A `smelt seed --select <source-path>` invocation is a hard error.
10. The aggregate `sources.yml` file is no longer recognised. A `sources.yml` at project root produces a clear migration error pointing at this spec.

## Known Divergences / Open Questions

- **Implementation lags spec.** This revision specifies the target shape. Today's implementation still uses DuckDB's `read_csv_auto`, the aggregate `sources.yml`, the `seed_paths` config knob, and the subdirectory-becomes-schema rule. A follow-up plan in `docs/plans/` migrates the implementation; until it lands, the implementation surface diverges from this spec on every Surface-section item.
- **Configurable DB-name mapping (`generate_alias_name` analogue).** Path-joined-with-underscore is a sensible floor, but real projects will want to override. A future spec — likely living next to or inside `smelt_yml.md` — defines a single mapping mechanism that applies to every kind (seed, source, model). Out of scope for this spec.
- **Tests on seed/source columns.** The sidecar YAML does not yet support a `tests:` key. The architecture spec defers `smelt.test` semantics to a future `tests.md`; when it lands, this YAML grows the column-level `tests:` shape uniformly with model column tests.
- **Drift diagnostic between CSV and pinned YAML.** The "Re-pin schema from CSV" LSP code action is in scope here, but the diagnostic that surfaces drift (column added/removed, inferred type drift) is implementation-deferred to the LSP plan.
- **Ephemeral seed size limits.** A 100k-row CSV declared `materialization: ephemeral` would generate a `VALUES` literal of dangerous size. A future row-count threshold (warn, then error) is open; today's spec leaves the choice to the user.
- **`view` / `materialized_view` materialization for seeds.** Not supported in v1. Possible if a concrete need emerges; would lower as `CREATE VIEW … AS SELECT * FROM (VALUES …)` or backend-equivalent.
- **Selector grammar for `smelt seed --select`.** Spec says "address path or unambiguous leaf". The full glob/wildcard story (e.g., `data.raw.*`) lives in `cli.md`; the seed selector inherits from there when it lands.
- **Migration tooling.** No `smelt migrate` command exists. A bundled examples migration and a documentation note are the v1 story; a tool is a follow-up plan.

## References

- **Code** (target after migration plan lands):
  - `crates/smelt-core/src/seeds.rs` — `discover_seeds`, sidecar YAML loader, type inferencer, `SeedFile`/`SeedInfo`.
  - `crates/smelt-cli/src/seed.rs` — `smelt seed` orchestration.
  - `crates/smelt-cli/src/commands/seed.rs` — CLI entry.
  - `crates/smelt-backend/src/lib.rs` — `Backend::load_table` trait method.
  - `crates/smelt-backend-duckdb/src/lib.rs` — Appender-based ingest.
  - `crates/smelt-backend-spark/src/lib.rs` — `createDataFrame` ingest.
  - `crates/smelt-db/src/schema.rs` — sidecar YAML → `ModelSchema`.
  - `crates/smelt-lsp/src/lib.rs` — missing-sidecar diagnostic, "Pin schema" code action.
- **Tests**:
  - `crates/smelt-core/tests/seed_inference.rs` — type-inference rules per column shape.
  - `crates/smelt-core/tests/seed_yaml_validation.rs` — column-set mismatch, type coercion, nullable enforcement.
  - `crates/smelt-cli/tests/seed_loading.rs` — backend ingest end-to-end.
  - `crates/smelt-cli/tests/example_diagnostics.rs` — bundled examples have no diagnostics.
- **User docs**:
  - `docs-site/docs/guide/seeds.md` — user-facing seed guide.
  - `docs-site/docs/guide/sources.md` — user-facing source guide (shares the YAML shape).
  - `docs-site/docs/reference/smelt-yml.md` — `paths:` key reference.
- **Plans (history)**: `docs/plans/20260406-seed-schema.md` (compile-time inference), `docs/plans/20260502-smelt-loop-findings.md` Phase 3 (TB-2 close: temporal types). The migration plan implementing this revision is pending.
- **Related specs**:
  - `architecture.md` §"Resolution" — universal `smelt.<path>` addressing and within-directory uniqueness.
  - `architecture.md` §"Models as functions" — materialization axes (`table` / `ephemeral`).
  - `types.md` — `DataType` vocabulary the inferencer produces.
  - `smelt_yml.md` — `paths:` key (renamed from `model_paths`), `targets[*].schema`.
  - `cli.md` — `smelt seed` and `smelt build` lifecycle.
  - `sources.md` (future) — full source-declaration spec sharing this YAML shape.
  - `tests.md` (future) — column-level tests landing in this YAML.
