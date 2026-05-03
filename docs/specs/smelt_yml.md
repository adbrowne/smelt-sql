---
feature: smelt_yml
status: experimental
last_reviewed: 2026-05-04
owners: [andrew]
---

# Project Configuration (`smelt.yml`)

> **Scope.** Normative spec for the project-level configuration file at the root of every smelt workspace. The `smelt.yml` surface defines the project name, where to find models and seeds, the executable backend targets, and project-wide defaults. Per-feature configuration (`incremental:` shape, function `backends:` frontmatter, …) is owned by the feature's own spec; this spec covers the top-level keys, the precedence rules, and unknown-key handling. This is a stub — sections may be brief — but every section says something concrete. Per-key reference (`docs-site/docs/reference/smelt-yml.md`) is the user-facing details page; this spec is what that reference must agree with.

## Surface

### Top-level keys

| Key | Type | Required | Default | Meaning |
|-----|------|----------|---------|---------|
| `name` | string | yes | — | Project name. Decorative; appears in run logs. |
| `version` | integer | no | `1` | Schema version of the `smelt.yml` format. Optional to remove the trip-hazard where users wrote a semver string and got a parse error. Currently only `1` is meaningful. |
| `paths` | list of strings | no | `["models"]` | Workspace-relative directories scanned for project files (`.sql`, `.py`, `.csv`, `.yml`). Kind is determined by file format/content (`architecture.md` §"Resolution"), not by which directory the file lives in. |
| `targets` | map of `<name>` → target object | yes | — | Named execution environments. The `--target` CLI flag selects one (default `dev`). |
| `default_materialization` | string | no | `"view"` | Project-level fallback materialization for any model that does not declare its own. Accepts `table`, `view`, `ephemeral`, `materialized_view`, `test`. |
| `models` | map of `<name>` → model-config object | no | `{}` | Per-model overrides keyed by model name. Each entry may declare `materialization`, `tags`, `target`, `incremental`, etc. |
| `python` | string | no | — | Path to a Python interpreter for Python-model discovery. The `SMELT_PYTHON` environment variable takes precedence. |
| `unstable_schema` | bool | no | `false` | Gate for unstable feature surfaces (e.g. `provenance:` and `joins:` frontmatter keys in `functions.md`). When `true`, the gated keys parse without warnings. |

The full per-key reference (target sub-shape, model-config sub-shape, incremental config, schema-evolution config) is in `docs-site/docs/reference/smelt-yml.md`. This spec mirrors the implemented `Config` struct in `crates/smelt-core/src/config.rs`; anything beyond what that struct accepts is "shape TBD" until it lands.

### Target shape (per `targets.<name>`)

| Field | Type | Required | Meaning |
|-------|------|----------|---------|
| `type` | string | yes | Backend type — `duckdb` or `spark`. |
| `schema` | string | yes | Schema name used for materialised tables/views and target-schema seeds. |
| `database` | string | DuckDB only | Path to the `.duckdb` file (relative to project root). |
| `connect_url` | string | Spark only | Spark Connect URL (e.g. `sc://localhost:15002`). |
| `catalog` | string | Spark only | Optional Spark catalog name. |
| `warehouse` | string | Spark only | Base directory for file-based output (Parquet warehouse). |
| `format` | string | Spark only | `delta` (default) or `parquet`. Affects schema-evolution capabilities. |

### Model-config shape (per `models.<name>`)

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `materialization` | string | inherits `default_materialization` | Override per model. |
| `incremental` | object | absent | Incremental-materialization config — full shape in `incremental_models.md`. |
| `tags` | list of strings | `[]` | Selector tags merged with frontmatter tags (union, deduplicated). |
| `target` | string | inherits CLI `--target` | Override which target this model executes on. |

### Precedence rules

1. **Materialization**: SQL frontmatter `materialization:` > `smelt.yml` `models.<name>.materialization` > `smelt.yml` `default_materialization` > built-in default `view`.
2. **Incremental**: SQL frontmatter > `smelt.yml::models.<name>.incremental`.
3. **Target**: SQL frontmatter `target:` > `smelt.yml::models.<name>.target` > CLI `--target` flag (default `dev`).
4. **Tags**: union of `smelt.yml::models.<name>.tags` and frontmatter `tags`, deduplicated.

### Unknown keys

An unknown top-level key produces a warning, not an error. The file otherwise parses normally. This is intentional — adding a key is non-breaking, and rejecting unknown keys would force a lockstep between the `smelt.yml` parser and every consumer of the config struct.

A typo'd known key (e.g. `default_matrialization`) is currently silently ignored as "unknown" — escalating typos to errors is open (see Known Divergences).

## Semantics

1. **`smelt.yml` must exist at the workspace root.** Absence is a hard error from any subcommand that loads the project. This is the only filesystem invariant the architecture spec mandates (`architecture.md` §"Project layout").
2. **`name` is decorative.** It appears in run logs but is not used as a schema name, table name, or namespace component. Renaming the project is safe.
3. **`version: 1` is the only accepted value today.** Omitting `version` parses as `1`; supplying a string (`"0.1.0"`) is rejected with a deserialisation error so the user-mistake (mirroring `pyproject.toml`) surfaces clearly.
4. **`default_materialization` is `view` by default.** This is the implementation default in `Config::default_materialization()`. Users who want every model materialised as a table must set `default_materialization: table` explicitly (or annotate per-model).
5. **`paths` defaults to `["models"]` and is the single scan list.** Smelt walks every listed directory recursively, classifying each file by format/content (`.sql` bare-SELECT → model, `.sql` `smelt.define` → function, `.csv` → seed, `.yml` with sibling `.csv` → sidecar, `.yml` standalone → source). A listed directory that does not exist is silently skipped — no error.
6. **`unstable_schema: true` opts in to gated keys.** The list of currently-gated keys is owned by the feature spec that introduces them (today: `joins:` and `provenance:` in `functions.md`). The flag itself is parsed by a small text-based helper (`parse_unstable_schema_flag`) so even malformed `smelt.yml` files can be probed for the gate.
7. **Forward compatibility via warnings.** A new release that adds a top-level key produces a warning on older consumers, not an error — the old binary keeps working with the field absent. This invariant means tooling can read newer projects without crashing.

## Design

**`name` and `version` are decorative because identity is the directory.** Earlier shapes used `name` as a namespace component (e.g. `<name>.<schema>.<table>` qualified paths). That conflated workspace identity with database identity and made renames painful — every downstream model had to update its qualified references. Today the workspace's identity is its filesystem location, the directory containing `smelt.yml`; `name` is informational. This composes with the universal `smelt.<path>` addressing scheme (`architecture.md`) — paths refer to entities by location, not by project name.

**`default_materialization: view`.** A view-by-default doctrine keeps development cheap (no table-rebuild cost on every change) and pushes the user to opt in to `table` exactly where performance matters. Earlier discussions floated `table` as the default, but that punishes the iteration loop where most models are still being shaped. The view default is consistent with the dbt convention and matches the implementation. (Notably, the smelt-loop finding DG-9 originally suggested the default was `table`; that was an artefact of the loop agents' discovery process — the canonical default is `view`.)

**`unknown keys = warning, not error`.** The forward-compatibility invariant in Semantics §7 is the load-bearing reason. Strict-rejection of unknown keys forecloses staged rollouts (a new key cannot be authored in the project until every consumer recognises it). Warning-on-unknown lets new fields land in the parser ahead of every consumer recognising them, while still surfacing typos to the user.

**Per-feature config lives in feature specs.** `incremental:` shape, function frontmatter keys (`deterministic`, `idempotent`, …), schema-evolution config, and the per-column `tests:` map are owned by their feature specs (`incremental_models.md`, `functions.md`, future `schema_evolution.md`). This spec lists only the top-level shape and the cross-feature precedence rules; otherwise, the page would have to be re-written every time a new key lands.

**One scan list, not per-kind.** Earlier the config had `model_paths` and `seed_paths` as separate lists, with `sources.yml` declaring sources at the project root regardless. That conflated *where to look* with *what kind of thing is here* — yet `architecture.md` §"Resolution" already says kind is determined by file format/content, not by directory. Collapsing to a single `paths:` list aligns the config with the resolver's actual behaviour: the user picks where their project files live; smelt classifies them on read. Co-locating a `models/payments/seeds/lookup.csv` with the SQL that consumes it is now a layout choice rather than a config fight.

## Constraints & Invariants

1. The implemented `Config` struct in `crates/smelt-core/src/config.rs` is the source of truth for the parser. The Surface table above must match the struct's serde-deserialisation contract.
2. `default_materialization` defaults to `Materialization::View` — never `Table` — until a deliberate spec change moves the default.
3. Unknown top-level keys produce a warning, not an error. Strict-rejection requires a spec change.
4. `version` is an integer — string values are rejected at parse time.
5. The `unstable_schema:` gate is read by a separate text-based helper so it remains probeable on otherwise-malformed configs.

## Known Divergences / Open Questions

- **Typo escalation.** Unknown top-level keys are warned; whether typos of known keys (`default_matrialization`) should also warn or escalate to errors is open. A future schema-checker pass could fuzzy-match against the known-key set and emit a hint; not implemented today.
- **Per-key reference drift.** The user-facing reference (`docs-site/docs/reference/smelt-yml.md`) currently documents some fields this spec does not yet cover (`schema_evolution`, `columns`). The reference is ahead of the spec on those keys; when the corresponding feature specs land they will absorb those fields.
- **Multi-target precedence with frontmatter `target:`.** The model-level `target:` frontmatter overrides `smelt.yml::models.<name>.target`, which overrides the CLI `--target`. The frontmatter form is a relatively recent addition; whether it should also be allowed to declare a target *not* defined in `smelt.yml::targets` is open (today: hard error before any work begins).
- **`unstable_schema:` discoverability.** There is no `smelt unstable_schema list` or similar way to enumerate the currently-gated keys. Users find them through individual feature docs; a discoverability mechanism is open.

## References

- **Code**: `crates/smelt-core/src/config.rs` — `Config`, `Target`, `ModelConfig`, `Materialization`, `parse_unstable_schema_flag`, `parse_active_backends`, `parse_with_warnings`. The default functions are `default_paths` (for the unified `paths:` list) and `default_materialization`.
- **Tests**: `crates/smelt-core/src/config.rs::tests` — round-trip tests for materialization defaults, version handling, target shape, validation rules.
- **User docs**: `docs-site/docs/reference/smelt-yml.md` — per-key reference; `docs-site/docs/concepts/project-structure.md` — orientation page.
- **Plans (history)**: `docs/plans/20260502-smelt-loop-findings.md` — the spec-authoring plan that produced this stub (DG-4 close).
- **Related specs**:
  - `architecture.md` — `smelt.yml` is the workspace marker; `paths:` is the directory list scanned by the discovery layer; kind-by-content is owned there.
  - `incremental_models.md` — `incremental:` shape on `models.<name>`.
  - `functions.md` — `unstable_schema: true` gates `joins:` and `provenance:` frontmatter.
  - `seeds.md` — CSV parsing rules and `smelt seed` lifecycle; consumes `paths:` and `targets.<name>.schema`.
  - `sources.md` — source declaration shape; consumes `paths:` for discovery.
  - `cli.md` — `--target` resolves against `targets`; `--project-dir` is the workspace root.
