---
feature: seeds
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Seeds

> **Scope.** Normative spec for CSV seed loading: CSV format, type inference, the `smelt seed` lifecycle, ephemeral seed semantics, and seed-specific LSP tooling. The shared YAML grammar (used for both seed sidecars and standalone source declarations) is owned by `sources.md`. Discovery, addressing, default DB-name mapping, and the `Backend::load_table` ingest path are owned by `architecture.md`.

## Surface

### What a seed is

A `.csv` file under any directory listed in `smelt.yml::paths` is a **seed**. Smelt parses the CSV, infers (or reads from a sibling `.yml`) the column schema, and loads the data into the active backend on `smelt seed` / `smelt build`. The address (`smelt.<path>`) and default DB location (`<target_schema>.<path-joined-by-_>`) follow the universal rules in `architecture.md` §"Resolution" and §"Default materialization name mapping".

A `.yml` file with the same stem in the same directory is a **sidecar** to the seed (`architecture.md` §"Resolution"); it declares column types, descriptions, nullability, and `materialization:` for the seed. The shared YAML grammar lives in `sources.md` §"Source YAML shape"; the seed-specific extensions are listed below.

A `.yml` file with no sibling `.csv` is a **source**, not a seed. See `sources.md`.

### Sidecar YAML — seed-specific keys

A seed sidecar uses the source YAML shape (`sources.md` §"Source YAML shape") with two seed-only differences:

| Key | Required | Default | Meaning |
|-----|----------|---------|---------|
| `columns` | no | absent (full inference) | When present, declares the schema and pins types. When absent, smelt infers types from the CSV (see "Type inference"). The column set must match the CSV header exactly. |
| `materialization` | no | `table` | `table` (default) or `ephemeral`. Materialization axis is owned by `architecture.md` §"Two orthogonal axes"; what each value means for a CSV is documented in "Materialization for seeds" below. |
| `name` | — | — | **Not allowed on a seed.** A seed's database name is derived from its workspace path (`architecture.md` §"Default materialization name mapping"). Configurable mapping is future work. |

A YAML carrying only `description:` (no `columns:`, no `materialization:`) is valid for a seed — the description applies on top of full inference.

### Materialization for seeds

| Value | Effect |
|---|---|
| `table` (default) | `smelt seed` loads the CSV into `<target_schema>.<path-joined-by-_>` via `Backend::load_table(...)` (`architecture.md` §"Backend trait surface"). |
| `ephemeral` | No table is created; at compile time, references to the seed are rewritten to a CTE whose body is a `VALUES (...)` literal carrying the seed's data. |

`view` and `materialized_view` are not currently supported for seeds and produce a hard error at load time.

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

When a sidecar YAML does not declare `columns:`, smelt infers each column's type from the CSV data. Two phases consume the same inference rules:

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

### `smelt seed` lifecycle

For each discovered seed (sorted by full address path), in order:

1. `CREATE SCHEMA IF NOT EXISTS <target_schema>`.
2. `DROP TABLE IF EXISTS` / `DROP VIEW IF EXISTS` against the target name.
3. Parse the CSV; validate against the sidecar YAML if present; convert to Arrow `RecordBatch`es.
4. `Backend::load_table(schema, name, arrow_schema, batches)` — backend-specific ingest (`architecture.md` §"Backend trait surface").

Seeds with `materialization: ephemeral` are skipped — they have no table to create.

The seed phase of `smelt build` runs the same lifecycle before any model executes.

### `smelt seed` CLI

| Flag | Meaning |
|---|---|
| `--select <smelt-path>` | Load only the named seed (e.g., `--select data.raw.users`). |
| `--select <leaf>` | Match by leaf name when unambiguous. |
| `--show-results` | Print a 5-row preview after each load. |
| `--target <name>` | Override the active target. |

`--select` against a source path is a hard error ("not a seed"). Selector grammar (globs, wildcards) is owned by `cli.md`.

### LSP integration

- **Diagnostic on missing sidecar YAML**: a CSV without a sibling `.yml` emits a workspace warning ("Seed schema is inferred and may drift if the CSV changes — pin it"). Severity: warning, not error. Resolved when a sidecar is added.
- **Code action: "Pin schema to sidecar YAML"**: runs the inferencer, writes the result to a sibling `.yml` next to the CSV. Resolves the warning above.
- **Code action: "Re-pin schema from CSV"** (deferred): when a sidecar exists but its column set differs from the CSV's, re-run the inferencer and overwrite. Spec'd here, implementation-deferred.
- **Hover**: column descriptions from the sidecar YAML appear on hover over a column name in a model that references the seed.
- **Goto-definition**: `smelt.<path>` resolves to the CSV file. (For sources, it resolves to the YAML — see `sources.md`.)

## Semantics

1. **Schema-set agreement is mandatory when pinned.** When a sidecar declares `columns:`, the column set must match the CSV header exactly — same names, in any order, with no extras on either side. Mismatch → hard error at load time and a diagnostic at compile time. There is no "load only declared columns" or "infer the rest" mode.

2. **Type-coercion failures are hard.** Any value that does not parse as its declared/inferred type aborts the load with a file/row/column pointer. Smelt does not silently substitute NULL.

3. **`nullable: false` is a load-time check.** A NULL row value in a non-nullable column is a hard error. Compile-time type-checking treats the column as non-NULL when reasoning about downstream models.

4. **Idempotence.** Re-running `smelt seed` brings the database to the same state for the same `(CSV, YAML)` inputs. Re-loading replaces the table; existing rows are not appended.

5. **Compile-time and runtime inference agree on every recognised type.** Where they differ, runtime is the wider view (it sees every row, not just the first 100). When the two disagree on a recognised type, smelt emits a diagnostic at compile time so the user can pin the schema to the runtime-correct type.

6. **Empty cell is always NULL.** This rule is uniform across all column types, including `VARCHAR`. Users who need a literal empty string materialise it via `COALESCE(col, '')` in a downstream model.

7. **`materialization: ephemeral` desugars at compile time.** When a model references an ephemeral seed, the seed body is emitted as `VALUES (…)` (with explicit per-column `CAST` to preserve type fidelity) and the reference is rewritten to a CTE. No table is created; `smelt seed` does nothing for ephemeral seeds. Cross-backend: the printer is responsible for any per-dialect adjustments to the `VALUES` form.

8. **Discovery order is deterministic.** Seeds are loaded sorted by full address path (`smelt.<path>`). This makes runs reproducible and CI-friendly.

## Design

**Smelt owns CSV parsing and inference.** Earlier the loader called DuckDB's `read_csv_auto`, and a separate compile-time inferencer ran in `smelt-core`. The two could disagree (the historical TB-2 class), the runtime path was DuckDB-only, and Spark seeds had to be loaded through a side channel. Owning the parser collapses both inferencers into one, makes seeds backend-portable via `Backend::load_table(...)`, and removes the entire "two views must agree" surface area. The implementation uses the `csv` crate for tokenisation/quoting (battle-tested, small, no opinionated inference of its own) and a smelt-owned inferencer that produces Arrow `RecordBatch`es. The earlier DuckDB-driven loader is removed.

**Strict CSV defaults, no per-seed override surface in v1.** dbt and sqlmesh expose delimiter / quote / NULL-marker / header config per-seed. We deliberately don't, because (a) the spec stays small, (b) every override forces decisions about how it interacts with the inferencer, and (c) projects with non-standard CSVs can convert them at the source. When concrete need emerges, overrides land in the sidecar YAML — the file is already there.

**Empty cell is always NULL.** The DuckDB-inspired alternative ("empty is NULL for non-text, empty-string for VARCHAR") matches a user's mental model coming from DuckDB's `read_csv_auto`, but introduces a type-dependent rule that surprises users coming from anywhere else and requires the loader to track per-column type while parsing. Uniform "empty = NULL" is one rule, easy to explain, easy to implement, and trivially worked around with `COALESCE` downstream. The cost is that a literal empty string in a CSV cell cannot survive into a `VARCHAR` column; we judge that a small enough loss to pay for the simpler rule.

**`DECIMAL(p,s)` in the inferred type set, capped at `(18, 4)`.** With smelt owning inference, the rule is deterministic: any column with a fractional component that fits within `DECIMAL(18, 4)` and is not in scientific notation infers as DECIMAL; otherwise DOUBLE. Pure-integer columns are caught earlier as INTEGER. The cap exists so wide-fixed-point values don't accidentally pin a 38-digit DECIMAL from a 100-row sample.

**No `TIMESTAMP WITH TIME ZONE` inference.** A column with mixed-zone timestamps cannot be inferred safely from text alone — the timezone of `2025-01-10 08:00:00` is ambiguous. Recognising `Z` / `+00` suffixes and emitting `TIMESTAMP WITH TIME ZONE` is technically possible but invites silent bugs when rows mix zones or the producer changes format. Falling back to VARCHAR forces an explicit `CAST` in a downstream model, which is exactly the right place for the user to declare their timezone intent. This trades convenience for correctness.

**LSP-driven schema pinning.** A pinned schema is the safer state — drift is impossible because every column comes from the YAML — but typing column declarations by hand is friction. The "Pin schema to sidecar YAML" code action removes that friction: smelt infers, the user one-click commits the result. The "no sidecar" warning then nudges users to take the safer path. This is the schema equivalent of `cargo fmt --emit files`: the right answer is mechanical, so smelt offers to write it.

**Rejected alternatives.**

- *Auto-detect delimiter/quote like DuckDB.* Surface that magic only matters when a user has a non-standard CSV; we'd rather have them convert it explicitly or override in the (future) sidecar config.
- *Tests on seed columns now.* The architecture spec defers `smelt.test` semantics; reserving a `tests:` key now makes promises we cannot keep. Tests land in the shared YAML when the tests spec exists.
- *`view` and `materialized_view` materialization for seeds.* A view backed by `VALUES` is technically possible but offers little over `ephemeral` (inline) or `table` (real). Out of scope until a concrete need emerges.

## Constraints & Invariants

1. The compile-time and runtime CSV inferencers are the same code path with different sample sizes. They cannot diverge by construction.
2. The CSV parser is strict: comma, double-quote, mandatory header, UTF-8, empty cell = NULL. No auto-detection.
3. The inferred type set is exactly `{BOOLEAN, INTEGER, DECIMAL(p,s), DOUBLE, DATE, TIMESTAMP, VARCHAR}`. `TIMESTAMP WITH TIME ZONE`, `TIME`, `INTERVAL`, and complex types are never inferred from CSV.
4. When a sidecar YAML declares `columns:`, the CSV header column set must match exactly (by name); mismatch is a hard error.
5. Type-coercion failures during load (parse failure, NULL in `nullable: false` column) are hard errors with file/row/column pointers — never silent NULLs.
6. Seeds materialised as `table` are loaded via `Backend::load_table(...)`. There is no DuckDB-specific or Spark-specific shortcut.
7. Seeds with `materialization: ephemeral` are never loaded into the database; they are spliced as `VALUES (…)` at compile time.
8. The `name:` override is not allowed on a seed sidecar; seed DB names are derived from the address path (`architecture.md` §"Default materialization name mapping").

## Known Divergences / Open Questions

- **Implementation lags spec (partial).** The CSV parser, type inferencer, Arrow batch builder, `Backend::load_table` wiring (Phase 4), sidecar YAML parsing/validation, ephemeral seed CTE expansion, and materialization dispatch (Phase 5) are implemented. The LSP affordances — missing-sidecar diagnostic and "Pin schema" code action (Phase 7) — are implemented. Per-entity source YAMLs (Phase 6) are implemented; the aggregate `sources.yml` format is removed.
- **Drift diagnostic between CSV and pinned YAML.** The "Re-pin schema from CSV" LSP code action is in scope here, but the diagnostic that surfaces drift (column added/removed, inferred type drift) is implementation-deferred to the LSP plan.
- **Ephemeral seed size limits.** A 100k-row CSV declared `materialization: ephemeral` would generate a `VALUES` literal of dangerous size. A future row-count threshold (warn, then error) is open; today's spec leaves the choice to the user.
- **Tests on seed columns.** The shared YAML does not yet support `tests:`. Tests on seed/source/model columns will land together when `tests.md` exists.
- **`view` / `materialized_view` materialization for seeds.** Not supported in v1. Possible if a concrete need emerges; would lower as `CREATE VIEW … AS SELECT * FROM (VALUES …)` or backend-equivalent.
- **Migration tooling.** No `smelt migrate` command exists. A bundled examples migration and a documentation note are the v1 story; a tool is a follow-up plan.

## References

- **Code** (as of Phase 4):
  - `crates/smelt-core/src/seeds/` — module directory: `csv.rs` (strict reader), `infer.rs` (type inferencer), `arrow.rs` (Arrow batch builder), `error.rs` (`SeedError`), `mod.rs` (`discover_seed_infos`, `SeedInfo`).
  - `crates/smelt-cli/src/seed.rs` — `smelt seed` orchestration.
  - `crates/smelt-cli/src/commands/seed.rs` — CLI entry.
  - `crates/smelt-backend-duckdb/src/lib.rs` — `Backend::load_table` via Appender.
  - `crates/smelt-backend-spark/src/lib.rs` — `Backend::load_table` via `createDataFrame`.
  - `crates/smelt-db/src/schema.rs` — sidecar YAML → `ModelSchema` (shared with sources).
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
  - `architecture.md` §"Resolution" — universal `smelt.<path>` addressing, sidecar tiebreaker, cross-path uniqueness.
  - `architecture.md` §"Default materialization name mapping" — the rule seed DB names follow.
  - `architecture.md` §"Backend trait surface" — `load_table(...)` ingest path.
  - `architecture.md` §"Two orthogonal axes" — materialization framework.
  - `sources.md` — owns the shared YAML grammar; the no-load complement of this spec.
  - `smelt_yml.md` — `paths:` key consumed for discovery.
  - `types.md` — `DataType` vocabulary the inferencer produces.
  - `cli.md` — `smelt seed` and `smelt build` lifecycle.
