---
feature: models
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Models

> **What this is.** A normative spec for SQL model files — the core unit of computation in smelt. Covers file format, YAML frontmatter schema, model naming, materialization modes, and the `smelt.models.<name>` reference surface.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.

## Surface

### File format

A model is a `.sql` file discovered by recursively walking each directory listed in `model_paths` (default: `["models"]`). Files may be:

**Single-model** — the file contains one SQL query, optionally preceded by YAML frontmatter:

```sql
---
materialization: table
tags: [revenue, core]
---

SELECT order_date, SUM(amount) AS revenue
FROM smelt.models.orders
GROUP BY 1
```

**Multi-model** — the file defines multiple models using `--- name: model_name ---` section delimiters:

```sql
--- name: staging_orders ---
materialization: ephemeral
---
SELECT * FROM smelt.sources.raw.orders WHERE status != 'cancelled'

--- name: daily_revenue ---
materialization: table
---
SELECT DATE(order_time) AS order_date, SUM(amount) AS revenue
FROM smelt.models.staging_orders
GROUP BY 1
```

Each section delimiter must follow the exact form `--- name: <model_name> ---` (leading/trailing spaces around the name are trimmed). Any other `--- X ---` form is a hard parse error.

### Model naming

| File type | Name source |
|-----------|-------------|
| Single-model | `file_stem()` — the filename without extension (e.g. `daily_revenue.sql` → `daily_revenue`) |
| Multi-model | The `--- name: <model_name> ---` delimiter; the optional `name:` YAML key in the section body is ignored |

The `name:` frontmatter key in single-model files is accepted by the parser but has no effect on the model's identity; the file stem is always authoritative.

### YAML frontmatter keys

All keys are optional. Unknown keys are a **hard error** (`deny_unknown_fields` — the parser rejects them rather than silently ignoring them).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | — | Accepted but ignored in single-model files; overridden by delimiter in multi-model files |
| `materialization` | enum | project default (`view`) | How to persist the model's output. See Materialization modes. |
| `incremental` | object | — | Incremental configuration. See `incremental_models.md`. |
| `target` | string | — | Override execution target for this model (overrides `smelt.yml` and `--target`). Not valid on `ephemeral` or `test` models. |
| `tags` | string[] | `[]` | Organization labels. Merged with `smelt.yml` model config tags (union, deduplicated). |
| `owner` | string | — | Responsible team or person. Informational; surfaced in data catalog. |
| `description` | string | — | Human-readable model description. Surfaced in data catalog. |
| `columns` | object | `{}` | Per-column metadata. See Column metadata below. |
| `backend_hints` | object | `{}` | Freeform backend-specific hints (forward compatibility). Not validated. |
| `test` | object | — | Test specification. Only valid when `materialization: test`. See `testing.md`. |
| `schema_evolution` | object | — | Schema evolution strategy. See `schema_evolution.md`. |
| `format` | enum (`delta` \| `parquet`) | target default | Override table format. Ignored for DuckDB targets; Spark targets default to `delta`. |

### Materialization modes

| Value | Behavior |
|-------|----------|
| `view` | Creates a SQL view. No data stored; query re-evaluated on each read. Default if unset. |
| `table` | Persists the query result as a physical table. |
| `ephemeral` | Not materialized. SQL is inlined as a CTE into every downstream model that references it. |
| `materialized_view` | Backend-managed persistent view; the backend controls refresh scheduling. |
| `test` | Not materialized. The model defines a unit test. SQL body is a test query; `test:` key in frontmatter declares mock data and assertions. See `testing.md`. |

### Materialization precedence (highest to lowest)

1. YAML frontmatter in the `.sql` file
2. `models.<name>.materialization` in `smelt.yml`
3. `default_materialization` in `smelt.yml` (fallback: `view`)

### Column metadata

Under `columns:`, each key is a column name and each value is an object with:

| Key | Type | Description |
|-----|------|-------------|
| `description` | string | Column description; surfaced in data catalog |
| `data_latency` | object | Late-arrival configuration for incremental safety analysis |
| `tests` | list | Column-level test constraints (`not_null`, `unique`, `{accepted_values: [...]}`, etc.) |
| `default` | string | Raw SQL expression for the column's default value during schema evolution |
| `backfill` | string | Raw SQL expression for backfilling existing rows when a column is added |

### Reference syntax

Within model SQL, other models are referenced using `smelt.models.<name>`:

```sql
FROM smelt.models.upstream_model
FROM smelt.models.my_seed       -- seeds are valid ref targets
```

Named parameter syntax is parsed but **not executed at runtime**:

```sql
FROM smelt.models.events(filter => date > '2024-01-01', limit => 1000)
```

External sources are referenced using `smelt.sources.<schema>.<table>` (see `sources.md`).

### Constraint violations

| Combination | Result |
|-------------|--------|
| `ephemeral` + `incremental` | Hard error |
| `ephemeral` + `target` override | Hard error |
| `test` + `incremental` | Hard error |
| `test` + `target` override | Hard error |
| `view` + `incremental.enabled: true` | Warning (stderr); incremental config ignored |
| `materialized_view` + `incremental.enabled: true` | Warning (stderr); incremental config ignored |
| Unknown frontmatter key | Hard error (`deny_unknown_fields`) |

## Semantics

### Model discovery

smelt recursively walks each path in `model_paths` (resolved relative to the project root), following symlinks, and collects all `.sql` files. Files are parsed independently — multi-model files yield multiple `ModelFile` entries, one per section.

Model names must be unique across the project. If two models produce the same name (e.g., `models/users.sql` and `models/archive/users.sql`), behavior is undefined (last writer wins in the current implementation).

### Ephemeral inlining

An `ephemeral` model's SQL is substituted as a CTE at each point a downstream model references it via `smelt.models.<name>`. The ephemeral model itself is never materialized. If multiple downstream models reference the same ephemeral, each gets an independent copy of the CTE; there is no shared materialization.

Transitive ephemeral chains are resolved: if `a` references ephemeral `b`, which references ephemeral `c`, then `a` gets both `b` and `c` as CTEs.

### Tag merging

A model's effective tags are the deduplicated union of:
1. Tags listed under `models.<name>.tags` in `smelt.yml`
2. Tags listed in the model's `tags:` frontmatter

Order within the merged list is smelt.yml tags first, frontmatter tags second, with duplicates removed on second occurrence.

### Materialization change

When a model's effective materialization type changes between runs (e.g., `view` → `table`), smelt drops the existing database object (whatever type it currently is) and creates the new one. No manual `DROP` is needed.

### Unknown frontmatter fields

The YAML frontmatter parser uses `serde`'s `deny_unknown_fields` mode. Any key not in the table above produces a parse error that prevents the model from loading. This is intentional: typos in frontmatter keys (e.g., `materialized: table` instead of `materialization: table`) surface immediately rather than silently using defaults.

## Design

**File stem as identity.** Model names derive from file paths so the filesystem is the source of truth. Names don't need to be declared; they fall out of where you put the file. This aligns with the `smelt.<path>` addressing principle (`architecture.md`): identity is structural, not declared. The `name:` frontmatter key is accepted (so YAML frontmatter round-trips cleanly) but is not authoritative.

**Multi-model files.** The `--- name: model_name ---` syntax allows logically-related models to live in one file without requiring a directory hierarchy. This is useful for staging + mart pairs or small pipelines that belong together conceptually. The name must be in the delimiter (not just YAML body) so the file can be scanned without full YAML parsing.

**Five materialization modes, not three.** dbt has three modes; smelt adds `materialized_view` (backend-managed refresh lifecycle) and `test` (first-class test declaration). `test` as a materialization mode keeps test SQL in the same format as model SQL — no separate test file format — which means the parser, type checker, and LSP all work uniformly across models and tests.

**Tag union, not override.** Tags accumulate across config layers rather than overriding. This lets a project-level `smelt.yml` add organization-wide tags (e.g., `pii`, `sla`) to specific models without preventing model authors from adding their own. Override semantics would require model authors to re-declare all project-level tags whenever they add their own.

**`deny_unknown_fields`.** Strict field validation catches typos before execution. The alternative — silently ignoring unknown keys — hides configuration mistakes and makes frontmatter edits feel non-deterministic. The error message from `serde` names the offending field, which is sufficient for the user to correct it.

**Named parameters parsed but deferred.** `smelt.models.name(filter => ...)` syntax is parsed to avoid breaking the grammar if/when it is implemented. It is not executed today. The note in user docs is the authoritative statement of current status.

## Constraints & Invariants

1. **Every model file is pure SQL.** No Jinja, no conditionals, no `is_incremental()`. The framework injects time filters and other execution-time rewrites; the logical SQL is static.
2. **Ephemeral models have no database object.** They produce no `CREATE TABLE`, `CREATE VIEW`, or DDL of any kind. Their SQL exists only as text substituted into downstream models.
3. **Test models have no database object.** `materialization: test` models are executed in-memory against a mock dataset; they never produce persistent state.
4. **Model names are unique within a project.** The discovery pass must not yield two `ModelFile` entries with the same `model_name`.
5. **Unknown frontmatter keys are rejected.** The parser must not silently accept and ignore unknown YAML keys.
6. **Tags are additive.** No frontmatter tag can remove a tag assigned by `smelt.yml`.

## Known Divergences / Open Questions

- **`test` mode missing from materializations user guide.** `docs-site/docs/guide/materializations.md` documents four materialization types; `test` is absent. It is documented only in the testing guide. Should be added to materializations page.
- **Duplicate model names undefined.** If `models/users.sql` and `models/archive/users.sql` both exist, the current implementation uses last-discovery order with no diagnostic. The spec mandates uniqueness; the implementation should emit an error.
- **`name:` in single-model frontmatter is ignored but accepted.** This is technically inconsistent (the field is silently dropped). A future cleanup could either remove support for it or make it an alias for renaming the model (which would conflict with file-stem identity).
- **Named parameter syntax in `smelt.models.<name>(...)`.** Parsed, not executed. Tracked in user docs as a note; no implementation timeline.
- **`backend_hints` is completely unvalidated.** Any freeform YAML is accepted. No backend currently reads it. It is a forward-compatibility escape hatch.

## References

- **Code**:
  - `crates/smelt-core/src/metadata.rs` — `ModelMetadata`, `FileMetadata`, `extract_file_metadata()`
  - `crates/smelt-core/src/model_id.rs` — `ModelId`, name-from-path derivation
  - `crates/smelt-core/src/discovery.rs` — `ModelDiscovery`, model file walking
  - `crates/smelt-core/src/config.rs` — `Materialization`, `ModelConfig`, `validate_model_configs()`, tag-merge logic
- **Tests**:
  - `crates/smelt-core/src/metadata.rs` (inline `#[cfg(test)]`) — frontmatter parsing, multi-model, unknown fields, format, schema evolution
  - `crates/smelt-core/src/config.rs` (inline `#[cfg(test)]`) — materialization validation, tag merging, ephemeral/test constraints
- **User docs**:
  - `docs-site/docs/guide/sql-models.md`
  - `docs-site/docs/guide/materializations.md` (missing `test` mode)
- **Related specs**:
  - `architecture.md` — `smelt.<path>` addressing scheme and identity-from-structure principle
  - `incremental_models.md` — incremental frontmatter keys
  - `testing.md` — `materialization: test` and the `test:` frontmatter key (forthcoming)
  - `schema_evolution.md` — `schema_evolution:` and `columns.default/backfill` frontmatter keys (forthcoming)
