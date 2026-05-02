---
feature: seeds
status: experimental
last_reviewed: 2026-05-03
owners: [andrew]
---

# Seeds

> **Scope.** Normative spec for CSV seed loading: filesystem layout, schema/table mapping, addressing in models, and compile-time vs. runtime type inference. The universal addressing rule (`smelt.<path>` everywhere) is owned by `architecture.md` §"Resolution"; this spec specialises it for `.csv` files. This is a stub — sections may be brief — but every section says something concrete.

## Surface

### Filesystem layout and table mapping

Seeds live under directories listed in `smelt.yml::seed_paths` (default `["seeds"]`). The CSV file's path under the seed directory determines two things: the qualified table name written to the database, and the `smelt.<path>` reference used in models.

| Filesystem location | Loaded into | Reference in models (per architecture.md design) |
|---------------------|-------------|---------------------------------------------------|
| `seeds/raw_orders.csv` | `<target_schema>.raw_orders` | `smelt.seeds.raw_orders` |
| `seeds/users.csv` | `<target_schema>.users` | `smelt.seeds.users` |
| `seeds/raw/orders.csv` | `raw.orders` | `smelt.seeds.raw.orders` |
| `seeds/raw/users.csv` | `raw.users` | `smelt.seeds.raw.users` |

`<target_schema>` comes from the active target's `schema:` key in `smelt.yml` (default `main`). The first path component under `seeds/` (when present) is the database **schema** for the loaded table; the file stem is the table name. Subdirectory seeds are equivalent to schema-qualified seeds.

**Implementation divergence (current, before resolution-unification work).** The implementation today maps top-level seeds and subdirectory seeds onto the **model** and **source** addressing namespaces respectively (because the loader's qualified name already lives in the target schema for one and a separate schema for the other). In existing examples and the `examples/timeseries/` workspace, a top-level seed appears as `smelt.models.<name>` and a subdirectory seed appears as `smelt.sources.<schema>.<name>`. This is a divergence between this spec / `architecture.md` (which specifies path-derived `smelt.seeds.<path>` per the universal addressing scheme) and the current resolver. Closing this divergence is a follow-up plan; until it lands, the implementation surface is documented in Known Divergences below.

### Compile-time vs. runtime type inference

Seed schemas are inferred twice and the two inferencers can disagree.

- **Runtime (DuckDB `read_csv_auto`).** Recognises `INTEGER`, `BIGINT`, `DOUBLE`, `BOOLEAN`, `DATE`, `TIMESTAMP`, `VARCHAR`. The materialised table on disk uses these types.
- **Compile-time (`smelt-db` schema extraction; consumed by the LSP and `smelt table`).** A simpler inferencer that recognises only `BOOLEAN`, `INTEGER`, `DOUBLE`, `Text`. Date- and timestamp-shaped columns are reported as `Text`. Columns the inferencer cannot classify default to `Text`.

`smelt table <model>` reports the **compile-time** schema (what type-checking sees), not DuckDB's runtime schema (what `DESCRIBE` would print). The two are intentionally aligned for the runtime types the compile-time inferencer recognises (`BOOLEAN`, `INTEGER`, `DOUBLE`); they diverge for `DATE` and `TIMESTAMP`, which the compile-time inferencer does not yet detect.

### `smelt seed` lifecycle

For each discovered seed CSV:

1. `CREATE SCHEMA IF NOT EXISTS <schema>` (target or sub-directory schema).
2. Drop any existing table or view of the same qualified name.
3. `CREATE TABLE <schema>.<name> AS SELECT * FROM read_csv_auto('<path>')`.

Seeds are loaded sequentially in deterministic (sorted-by-qualified-name) order. The seed step is shared by `smelt seed` and the seed phase of `smelt build`.

## Semantics

1. **Filesystem path is identity.** A seed's qualified table name and `smelt.<path>` reference are derived from its location under `seed_paths` — no manifest, no per-seed YAML. A rename or move changes the call surface (same as for models).
2. **Subdirectory = schema.** Exactly one subdirectory level under a `seed_paths` entry is taken as a schema name; deeper nesting is not yet supported (Known Divergences). Top-level CSVs land in the target's schema.
3. **Idempotence.** Re-running `smelt seed` reloads each table from its CSV — old contents are replaced, not appended. The runtime schema may change between runs (e.g., a new column appears in the CSV); this is the normal seed-development workflow.
4. **Compile-time inference may understate runtime types.** If `smelt table <staging>` shows `Text` for a column DuckDB's `read_csv_auto` types as `DATE`, the model's compile-time schema is the narrower view. Downstream models that consume this column inherit the compile-time `Text` typing, and an `ORDER BY` or `BETWEEN` against another `Date` value will diagnose `TypeMismatch` until an explicit `CAST` bridges the families. This is the user-visible failure mode of TB-2.
5. **Workaround until TB-2 is fixed.** In the first staging model that reads a temporal seed column, emit an explicit `CAST(col AS DATE)` (or `CAST(col AS TIMESTAMP)`). Downstream models then see the temporal type and the type checker is happy. The `CAST` is a no-op at runtime because the underlying DuckDB column is already `DATE`/`TIMESTAMP`.

## Design

**Seeds are addressed by path, like every other project entity.** The universal addressing scheme (`smelt.<path>`, `architecture.md` §"Resolution") is the rule; seeds are not an exception. A seed promoted to a SQL model (or a model demoted to a CSV) is a rename, not a callsite-rewriting refactor. The current implementation's mapping into `smelt.models.*` and `smelt.sources.*` predates the unification and is being walked back; the spec describes the intended steady state, with the discrepancy logged in Known Divergences.

**Two inferencers exist deliberately.** The compile-time inferencer is in `smelt-db` (a sync, pure module that the LSP and CLI both consume); pulling in DuckDB at compile time would tie the compile-time stack to a heavyweight runtime dependency and is out of scope for the LSP. Keeping a simpler inferencer compile-side and DuckDB's `read_csv_auto` runtime-side is the correct factoring, but the two must agree on the types they both claim to support. TB-2 is a coverage gap (compile-time inferencer does not recognise `DATE`/`TIMESTAMP` shapes), not a deliberate choice — the fix in `docs/plans/20260502-smelt-loop-findings.md` Phase 3 closes the gap.

**`smelt table` reflects the compile-time schema.** A user inspecting "what does smelt think the columns of this model are?" wants the compile-time view, because that is what type-checks every downstream model. Showing the runtime schema would mask the type-checker's view and make TB-2-class divergences invisible. When the compile-time inferencer is correct, the two views agree.

**No per-seed YAML.** Seeds inherit smelt's "configuration falls out of structure" doctrine: the path determines schema and table name, the CSV's content determines the columns, `read_csv_auto` determines runtime types. A future `seeds.yml` could pin types or describe relationships, but is not in scope today and would be additive (not breaking).

## Constraints & Invariants

1. A seed CSV's qualified table name (`<schema>.<name>`) is derived purely from its path; no per-seed metadata can override it.
2. Seed loading is idempotent: re-running `smelt seed` brings the database to the same state for the same set of CSV inputs.
3. Top-level CSVs go to the active target's `schema:`; subdirectory CSVs go to a schema named after the immediate parent directory.
4. Compile-time and runtime type inference must agree on the types the compile-time inferencer recognises (`BOOLEAN`, `INTEGER`, `DOUBLE`, `Text`); the divergence is exclusively in the compile-time inferencer's narrower coverage of temporal types.
5. The compile-time-inferred schema is the canonical schema for type-checking and `smelt table`; the runtime schema is opaque to type analysis.

## Known Divergences / Open Questions

- **Addressing inconsistency.** The architecture spec specifies path-derived `smelt.seeds.<path>` for every seed (consistent with the universal addressing scheme). The implementation today maps top-level seeds onto `smelt.models.<name>` (because they share the target schema with executed models) and subdirectory seeds onto `smelt.sources.<schema>.<name>` (because they pre-date the unified resolver). Existing user docs (`guide/seeds`) and example workspaces (`examples/timeseries/`) follow the implementation surface, not the architecture spec. The spec author chose the architecture-spec form here because it is the documented design; closing the divergence is a follow-up plan and the user docs will move once the resolver does. Cross-reference: `architecture.md` §"Resolution".
- **TB-2 — Passthrough `DATE` / `TIMESTAMP` seed columns infer as `Text` at compile time.** A seed CSV with a column of `2025-01-01` values is typed `DATE` by `read_csv_auto` at runtime, but the compile-time inferencer types it `Text`. A staging model `SELECT order_date FROM smelt.seeds.raw_orders` therefore emits a `Text` column at compile time and materialises as `VARCHAR` in DuckDB (because the cast wraps `Text`). Tracked in `docs/plans/20260502-smelt-loop-findings.md` Phase 3. Fix: extend the compile-time inferencer to recognise `YYYY-MM-DD` (`DATE`) and `YYYY-MM-DD HH:MM:SS` (`TIMESTAMP`) shapes (and verify `DECIMAL`-shaped columns map to `Double` at both layers). Once landed, this divergence moves into Semantics §4 ("compile-time and runtime inference agree for `BOOLEAN`, `INTEGER`, `DOUBLE`, `DATE`, `TIMESTAMP`; longer-form `DECIMAL` columns infer as `Double`").
- **No nested-subdirectory seed layout.** `seeds/<sub1>/<sub2>/foo.csv` is not specified; the discovery loop only descends one level. Whether deeper paths should produce dotted schema names (`<sub1>.<sub2>.foo`) or be rejected is open.
- **Seed type pinning.** Users have asked for a way to override the inferred runtime type of a seed column (e.g., force `DECIMAL(10,2)` instead of `DOUBLE`). The current loader has no override; a future `seeds.yml` could add it. Not in scope here.

## References

- **Code**:
  - `crates/smelt-cli/src/seed.rs` — `discover_seeds`, `execute_seed`, `SeedFile`, `SeedType` (Source vs. Target classification)
  - `crates/smelt-cli/src/commands/seed.rs` — CLI entry point
  - `crates/smelt-db/src/schema.rs` — compile-time schema extraction for seed-backed models
  - `crates/smelt-core/src/config.rs` — `seed_paths` (`default_seed_paths()`)
- **Tests**: unit tests in `crates/smelt-cli/src/seed.rs::tests`; integration coverage via `examples/timeseries/seeds/` and the example-diagnostics test (`cargo test -p smelt-cli --test example_diagnostics`).
- **User docs**: `docs-site/docs/guide/seeds.md` — to be reconciled against this spec via Phase 6 of `docs/plans/20260502-smelt-loop-findings.md`.
- **Plans (history)**: `docs/plans/20260502-smelt-loop-findings.md` — the spec-authoring plan and the TB-2 fix.
- **Related specs**:
  - `architecture.md` §"Resolution" — universal `smelt.<path>` addressing.
  - `types.md` — compile-time `DataType` vocabulary the inferencer produces.
  - `smelt_yml.md` — `seed_paths`, `targets[*].schema` keys consumed here.
  - `cli.md` — `smelt seed` and `smelt build` lifecycle.
