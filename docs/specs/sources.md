---
feature: sources
status: experimental
last_reviewed: 2026-06-13
owners: [andrew]
---

# Sources

> **What this is.** Normative spec for source declarations: externally-managed tables that smelt does not load but can type-check and route in `FROM` positions. Sources share their YAML grammar with seed sidecars (`seeds.md`); this spec owns that shared grammar and the source-only semantics. This is a stub — sections are brief — but every section says something concrete.
>
> **Spec-first rule.** Edit this file before writing the implementation plan. The spec diff is the change description.
>
> **Timeless-oracle rule.** This spec describes the feature as if it has always existed. No plan-phase headings (`### Phase A — …`), no inline phase labels (`Meta list (Phase A)`), no plan-vocabulary status callouts (`[deferred to Phase E1]`) in §Surface, §Semantics, §Design, or §Constraints. Implementation status that needs naming goes in §Known Divergences (describe behaviour, link the plan; phase numbers tolerated only when paired with a plan link) or §References → Plans (history) (link plan files; do not describe their phase structure). See the Timeless-oracle rule in `CLAUDE.md` for the full rule and good/bad examples.

## Surface

### What a source is

A **source** is an external table that already exists in the target database, populated by some pipeline outside smelt. Smelt declares the source's schema, type-checks references, surfaces the columns in the LSP, and routes `smelt.<path>` references to the underlying `<schema>.<table>` — but it never runs `CREATE TABLE` or `INSERT` for the source. `smelt seed` does not touch sources.

### Filesystem layout

A source is declared by a `.yml` file in any non-excluded directory under the project root. (Discovery is project-wide; `smelt.yml::paths` only strips address prefixes, it does not gate which directories are scanned — see `architecture.md` §"Resolution".) The file must **not** have a sibling `.csv` with the same stem in the same directory — that would make the YAML a seed sidecar instead (`architecture.md` §"Resolution").

| File on disk (with `paths: ["models"]`) | Address |
|---|---|
| `models/sources/raw/users.yml` | `smelt.sources.raw.users` |
| `models/external/api/orders.yml` | `smelt.external.api.orders` |

The address path follows universal addressing (`architecture.md` §"Resolution"). The mapping from `smelt.<path>` to `<db_schema>.<db_table>` follows the default rule in `architecture.md` §"Default materialization name mapping" — `<target_schema>.<path-joined-by-_>` — unless the YAML provides a `name:` override (recommended whenever the external pipeline named the table differently). The override is **target-aware**: it may pin one external name for all targets, or supply a per-target map so `--target dev` and `--target prod` resolve the same source to different external schemas/tables (see §"Target-aware `name:` override").

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
| `name` | no | derived | Override the database-side name. **Target-aware** (see §"Target-aware `name:` override"): either a single `<schema>.<table>` literal applied to every target, or a per-target map `{ <target>: <schema>.<table>, … }` so different targets read different external schemas/tables. When absent, defaults to `<target_schema>.<address-path-joined-by-_>`. |
| `timeseries` | no | absent | Declares a time dimension on this source (`event_time_column`, `partition_column`, `granularity`). See `timeseries.md`. When present, the named columns must appear in `columns:` with date/timestamp-compatible types. |
| `mutation_profile` | no | absent (undeclared) | Declares the source's mutation shape: `append_only` (rows are only ever appended), `mutable` (rows may be updated/deleted in place — only a full re-scan sees every change), or `change_feed` (the source itself exposes a CDC/CDF; a run reads only the rows that changed since the last run). This is the one non-derivable world-fact on the input-consumption axis (`models.md` §"Input-consumption axis"; `model_properties.md` §"Catalogued inputs") — `smelt-logical`'s input-delta discovery reads it via `SourceShape::from_source_info`. An unrecognised value is a fail-loud `MalformedSource` parse error. When absent, the conservative default applies: a clocked source (`timeseries:` present) is window-forward, an unclocked source is snapshot-diff. |
| `source_lateness` | no | absent (zero) | Declares the source-lateness margin — the term of the reach split (`model_properties.md` §"Unified bound/reach derivation") — as an interval (`'2 hours'`, `'1 day'`, …), parsed via the same fail-loud interval grammar as `horizon_ceiling` (`model_maintenance.md`). A malformed value is a fail-loud `MalformedSource` parse error. |
| `materialization` | — | — | **Not allowed on a source.** Sources are externally managed; declaring a materialization is a hard error pointing at the seed sidecar shape. |

The YAML grammar is shared with the seed sidecar (`seeds.md` §"Sidecar YAML — seed-specific keys"). Differences: a source must declare `columns:`; a source must not declare `materialization:`; a source supports the `name:` override (because the external table's name is not always a function of the workspace path); a source may declare `timeseries:` (a seed sidecar may not — seeds are loaded by smelt and have no externally-imposed partition layout).

### Source with `timeseries:` declaration

A source declaring a time dimension opts in to being a pushdown target for downstream planner rules — incremental models reading the source will receive source-filter pushdown based on the declared partition column:

```yaml
description: Raw events feed; partitioned daily by event_date.
columns:
  - { name: event_id, type: BIGINT, nullable: false }
  - { name: event_ts, type: TIMESTAMP, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
  - { name: user_id, type: INTEGER, nullable: false }
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
```

Declaring `timeseries:` does not change how the source is loaded — sources remain externally managed. It only declares the partition shape downstream consumers may rely on.

### Target-aware `name:` override

The `name:` override is resolved against the **active target** so a single source declaration can point at different external tables in different environments. Two forms are accepted:

**Literal form** — one `<schema>.<table>` string, applied to every target:

```yaml
name: raw_cdc.users        # all targets read raw_cdc.users
```

**Per-target map** — keys are target names from `smelt.yml::targets`, values are `<schema>.<table>` literals:

```yaml
name:
  dev:  raw_cdc_dev.users
  prod: raw_cdc.users
```

Resolution rules:

- The map is keyed by the active `--target` name. When the active target has an entry, its `<schema>.<table>` is used verbatim (the literal's schema is **not** overridden by the target's `schema:`).
- A target with no entry in the map falls back to the default mapping (`<target_schema>.<address-path-joined-by-_>`) — the map only overrides the targets it names.
- The literal form is shorthand for a map whose single value applies to every target.
- A map value that is not a `<schema>.<table>` literal, or a map key that names no declared target, is a `MalformedSource` error.

The grammar lives here in `sources.md`; `smelt.yml` carries no source-name config key. `smelt_yml.md` references this section for the target-aware behaviour.

### Discovery and addressing

Sources are discovered alongside every other project file by walking `paths:`. Resolution rules (`architecture.md` §"Resolution"):

- A `.yml` file with no sibling `.csv` of the same stem → source.
- A `.yml` file with a sibling `.csv` → sidecar to that seed (not a source). See `seeds.md`.
- Two files resolving to the same address (anywhere in the project) → workspace-load error.

### LSP surface

- **Hover** on a `smelt.<path>` reference to a source → table description + column list with types and descriptions.
- **Goto-definition** → opens the source `.yml`.
- **Diagnostics** — references to columns not declared in the source YAML produce an "undeclared column" diagnostic, same as for any other typed table reference.
- **No "Pin schema" code action.** Sources have no data file to infer from; the YAML is hand-written.

### Diagnostic codes (owned by this spec)

The codes below are owned by `sources.md` — `lsp.md` mirrors them in its catalogue but defers the trigger contract here. (Diagnostic ownership is a per-spec rule that a future `diagnostics.md` registry will formalise; the rule today is "diagnostic codes live with the feature that owns them.")

| Code | Severity | Trigger |
|---|---|---|
| `MalformedSource` | Error | A source `.yml` parses as YAML but violates the shape above (e.g., missing `columns`, `materialization:` key present, malformed column entry, a `name:` override whose value is not a `<schema>.<table>` literal / whose per-target map names an undeclared target, an unrecognised `mutation_profile:` value, or a `source_lateness:` value that does not parse as an interval). |
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

**Why `name:` is target-aware.** Schema otherwise comes from the active target, so a single `<schema>.<table>` literal would pin one schema across every target — `--target dev` and `--target prod` would both read the same hardcoded external schema, defeating multi-target portability. Real pipelines stage the same logical feed in different schemas per environment (`raw_cdc_dev` vs `raw_cdc`). Making the override a per-target map (with the literal form as the all-targets shorthand) lets one source declaration follow the environment. A purely table-only override (schema always from the target) was considered and rejected: it cannot express the common case where the *schema* differs per environment, which is exactly where source portability breaks.

**`materialization:` not allowed on sources.** Sources are external by definition — there is no smelt-controlled materialization. A `materialization: ephemeral` source would mean "smelt should not assume this table exists" which is closer to a feature flag than a data shape, and we have no concrete need for it. If one emerges, the spec opens up.

## Constraints & Invariants

1. A `.yml` file with no sibling `.csv` is a source; with a sibling `.csv` it is a sidecar. The kinds are disjoint.
2. Sources are never loaded by `smelt seed` or `smelt build`.
3. `materialization:` on a source YAML is a hard error.
4. The source YAML grammar is a strict subset of the seed sidecar grammar plus the source-only `name:` override.
5. The cross-path uniqueness rule (`architecture.md` §"Resolution") applies — a source's address is unique across all `paths:` roots.
6. Aggregate `sources.yml` at the project root is being retired in favour of per-entity source YAMLs. Once the legacy fallback is removed, its presence will produce a clear migration error; until then it is still parsed as a fallback (see Known Divergences).
7. **A `smelt.sources.<path>` reference resolves by its path prefix, not a separate namespace.** Addressing is the single `smelt.<path>` scheme (`architecture.md`); `sources.` is just the leading path segment, not a distinct namespace. A model whose leaf name happens to collide with a source's leaf segment does not shadow the source schema for that reference; the path prefix is dispositive. For example, when both `models/orders.sql` and `models/sources/raw/orders.yml` exist in the same project, `FROM smelt.sources.raw.orders` uses the source's declared column types — not the model's inferred schema — regardless of evaluation order.

## Known Divergences / Open Questions

- **Aggregate `sources.yml` presence is not yet a migration error (Constraint 6).** A present aggregate `sources.yml` is still parsed and consumed as a legacy type-information fallback when a project declares no per-entity sources. A malformed aggregate file surfaces a `YamlParseError` (Warning) diagnostic, but the presence-is-an-error rule awaits removal of the legacy fallback. Tracked as BUG-078 in `docs/bug-hunt/2026-05-30-findings.md`.
- **Source-existence verification.** A future `smelt verify` pass could check that every declared source exists in the target database with the declared columns. Out of scope here.
- **Column-level tests on sources.** Same status as for seeds — column-level tests on the shared YAML grammar are not yet defined; `testing.md` covers `smelt.test` declarations but not per-column assertions on a source's sidecar. The shared YAML grammar will grow uniformly when that surface is added.
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
  - `timeseries.md` — declares the `timeseries:` block grammar this spec hosts on external sources.
  - `batched_models.md` — primary consumer of `timeseries:` on sources, via source-filter pushdown.
  - `types.md` — `DataType` vocabulary used by `columns[].type`.
