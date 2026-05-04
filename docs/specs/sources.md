---
feature: sources
status: experimental
last_reviewed: 2026-05-04
owners: [andrew]
---

# Sources

> **Scope.** Normative spec for source declarations: externally-managed tables that smelt does not load but can type-check and route in `FROM` positions. Sources share their YAML grammar with seed sidecars (`seeds.md`); this spec owns that shared grammar and the source-only semantics. This is a stub — sections are brief — but every section says something concrete.

## Surface

### What a source is

A **source** is an external table that already exists in the target database, populated by some pipeline outside smelt. Smelt declares the source's schema, type-checks references, surfaces the columns in the LSP, and routes `smelt.<path>` references to the underlying `<schema>.<table>` — but it never runs `CREATE TABLE` or `INSERT` for the source. `smelt seed` does not touch sources.

### Filesystem layout

A source is declared by a `.yml` file under any directory listed in `smelt.yml::paths`. The file must **not** have a sibling `.csv` with the same stem in the same directory — that would make the YAML a seed sidecar instead (`architecture.md` §"Resolution").

| File on disk (with `paths: ["models"]`) | Address |
|---|---|
| `models/sources/raw/users.yml` | `smelt.sources.raw.users` |
| `models/external/api/orders.yml` | `smelt.external.api.orders` |

The address path follows universal addressing (`architecture.md` §"Resolution"). The mapping from `smelt.<path>` to `<db_schema>.<db_table>` follows the default rule in `architecture.md` §"Default materialization name mapping" — `<target_schema>.<path-joined-by-_>` — unless the YAML provides a `name:` override (recommended whenever the external pipeline named the table differently).

### Source YAML shape

```yaml
description: Raw user dimension; populated nightly by the CDC pipeline.
columns:
  - name: user_id
    type: INTEGER
    nullable: false
    description: Surrogate key.
  - name: user_name
    type: VARCHAR
  - name: signup_date
    type: DATE
```

| Key | Required | Default | Meaning |
|-----|----------|---------|---------|
| `description` | no | absent | Free-text description, surfaced in LSP hover. |
| `columns` | yes | — | Column declarations. Sources without a column list are not useful — type-checking has no contract to enforce. |
| `columns[].name` | yes | — | Column name as it appears in the database. |
| `columns[].type` | yes | — | Smelt `DataType` (`types.md`). Same vocabulary as model type-checking and seed sidecars. |
| `columns[].nullable` | no | `true` | Whether the column may contain NULL in the upstream database. Type-checking respects this. |
| `columns[].description` | no | absent | Free-text description, surfaced in LSP hover. |
| `name` | no | derived | Override the database-side name. When present, must be a `<schema>.<table>` literal. When absent, defaults to `<target_schema>.<address-path-joined-by-_>`. |
| `materialization` | — | — | **Not allowed on a source.** Sources are externally managed; declaring a materialization is a hard error pointing at the seed sidecar shape. |

The YAML grammar is shared with the seed sidecar (`seeds.md` §"Sidecar YAML — seed-specific keys"). The only differences are: a source must declare `columns:`; a source must not declare `materialization:`; a source supports the `name:` override (because the external table's name is not always a function of the workspace path).

### Discovery and addressing

Sources are discovered alongside every other project file by walking `paths:`. Resolution rules (`architecture.md` §"Resolution"):

- A `.yml` file with no sibling `.csv` of the same stem → source.
- A `.yml` file with a sibling `.csv` → sidecar to that seed (not a source). See `seeds.md`.
- Two files resolving to the same address (across scan paths) → workspace-load error.

### LSP surface

- **Hover** on a `smelt.<path>` reference to a source → table description + column list with types and descriptions.
- **Goto-definition** → opens the source `.yml`.
- **Diagnostics** — references to columns not declared in the source YAML produce an "undeclared column" diagnostic, same as for any other typed table reference.
- **No "Pin schema" code action.** Sources have no data file to infer from; the YAML is hand-written.

### Diagnostic codes (owned by this spec)

The codes below are owned by `sources.md` — `lsp.md` mirrors them in its catalogue but defers the trigger contract here. (Diagnostic ownership is a per-spec rule that a future `diagnostics.md` registry will formalise; the rule today is "diagnostic codes live with the feature that owns them.")

| Code | Severity | Trigger |
|---|---|---|
| `MalformedSource` | Error | A source `.yml` parses as YAML but violates the shape above (e.g., missing `columns`, `materialization:` key present, malformed column entry). |
| `SourceTypeError` | Error | A `columns[].type` value is not a recognised smelt `DataType` (`types.md`). |

## Semantics

1. **Sources are never loaded.** `smelt seed`, `smelt build` (seed phase), and any other ingest path skip sources entirely. A `smelt seed --select <source-path>` invocation is a hard error ("not a seed").
2. **Schema is the contract.** When a model references a source column, the smelt type-checker uses the YAML's declared type. A column not declared in the YAML is undeclared and produces a diagnostic, even if the column exists in the upstream database.
3. **Smelt does not validate that the source exists.** A reference to a non-existent source surfaces only at execution time as a backend error. (A future implementation may add a `smelt verify` pass that checks every source against the live database; out of scope for this spec.)
4. **Address-only references.** A source has no body for the planner to inspect — it is black-box, like an `extern`, but addressable by path rather than by bare name (`architecture.md` §"Two orthogonal axes").
5. **Discovery and uniqueness.** A source's address is its workspace path under `paths:`, with the scan-root prefix stripped. The cross-path uniqueness rule (`architecture.md` §"Resolution") applies.

## Design

**Two concepts (seed and source), one YAML grammar.** Seeds and sources have different lifecycles — smelt loads a seed; an external pipeline owns a source — and that distinction is real to users. But every other concern overlaps: column types, descriptions, hover, goto-definition, future tests. Sharing the YAML grammar means one parser, one schema-resolution path, and one set of LSP affordances. The kind is determined structurally (sibling CSV present?), not by a configuration toggle. (Decided in the Q1 design discussion that produced this revision; alternatives — full unification under one "input" concept; sidecars only without standalone sources — were rejected as either too aggressive or as losing the lifecycle distinction.)

**Per-entity YAML, not aggregate `sources.yml`.** The aggregate file violates universal addressing — every project entity should live at its addressed path. `models/sources/raw/users.yml` *is* `smelt.sources.raw.users`. Splitting the old `sources.yml` into per-entity files is a hard cut; pre-1.0 + the workspace's "no backward compatibility" doctrine means we don't ship a compat shim. A `smelt migrate` follow-up tool can mechanise the rewrite.

**Why `name:` is allowed on sources but not on seeds.** A seed's identity *is* its workspace path — smelt picks the database name. A source's identity is the external table the pipeline produces; smelt only declares it. The external name is not a function of the workspace layout, so the YAML must be able to override the default mapping. Disallowing `name:` for seeds keeps the "config falls out of structure" doctrine intact for the things smelt actually owns.

**`materialization:` not allowed on sources.** Sources are external by definition — there is no smelt-controlled materialization. A `materialization: ephemeral` source would mean "smelt should not assume this table exists" which is closer to a feature flag than a data shape, and we have no concrete need for it. If one emerges, the spec opens up.

## Constraints & Invariants

1. A `.yml` file with no sibling `.csv` is a source; with a sibling `.csv` it is a sidecar. The kinds are disjoint.
2. Sources are never loaded by `smelt seed` or `smelt build`.
3. `materialization:` on a source YAML is a hard error.
4. The source YAML grammar is a strict subset of the seed sidecar grammar plus the source-only `name:` override.
5. The cross-path uniqueness rule (`architecture.md` §"Resolution") applies — a source's address is unique across all `paths:` roots.
6. Aggregate `sources.yml` at the project root is no longer recognised; its presence produces a clear migration error.

## Known Divergences / Open Questions

- **Source-existence verification.** A future `smelt verify` pass could check that every declared source exists in the target database with the declared columns. Out of scope here.
- **Column-level tests on sources.** Same status as for seeds — column-level tests on the shared YAML grammar are not yet defined; `testing.md` covers `materialization: test` models but not per-column assertions on a source's sidecar. The shared YAML grammar will grow uniformly when that surface is added.
- **Co-location with seeds.** Worth noting: a `.yml` declaring a source can be co-located with seed CSVs in the same directory (different stems), since kind-by-content makes the directory layout independent of kind. Style guides may discourage mixing for readability; the resolver does not.

## References

- **Code** (target after migration plan lands):
  - `crates/smelt-core/src/sources.rs` — source discovery, YAML loader, `SourceInfo`.
  - `crates/smelt-db/src/schema.rs` — source YAML → `ModelSchema` (shared with seed sidecars).
  - `crates/smelt-lsp/src/lib.rs` — hover, goto-definition for source references.
- **Tests**:
  - `crates/smelt-core/tests/source_yaml.rs` — schema validation, `name:` override, kind tiebreaker against seed sidecars.
- **User docs**:
  - `docs-site/docs/guide/sources.md` — user-facing source guide.
  - `docs-site/docs/reference/sources-yml.md` — per-key YAML reference (to be reconciled with this spec by the migration plan).
- **Plans (history)**: the migration plan implementing this spec is pending; see also `docs/plans/20260403-sources-yml-live-updates.md` for the prior incremental work on the aggregate `sources.yml` shape (now superseded).
- **Related specs**:
  - `architecture.md` §"Resolution" — kind-determination, sidecar tiebreaker, cross-path uniqueness.
  - `architecture.md` §"Default materialization name mapping" — the rule the source `name:` override departs from.
  - `seeds.md` — shares the YAML grammar; the load-side complement of this spec.
  - `smelt_yml.md` — `paths:` key the discovery layer consumes.
  - `types.md` — `DataType` vocabulary used by `columns[].type`.
