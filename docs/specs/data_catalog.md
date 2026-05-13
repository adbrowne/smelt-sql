---
feature: data_catalog
status: experimental
last_reviewed: 2026-05-05
owners: [andrew]
---

# Data Catalog

> **What this is.** A normative spec for `smelt docs generate` — the data catalog output format (markdown and JSON), per-model page contents, column description sources, the tag index, and the embedded CLI docs system (`smelt docs list`, `smelt docs show`, `smelt docs path`).

## Surface

### `smelt docs generate`

```
smelt docs generate [--format markdown|json] [--output <dir>] [--select <selector>]
```

Generates a data catalog from the current project. The default format is `markdown`. The default output directory is `target/docs/`.

`--select` applies the standard selector syntax to limit which models are included. Models excluded by the selector are omitted from the output.

### `smelt docs list`

```
smelt docs list
```

Prints a sorted list of all embedded documentation topic paths to stdout. Topic paths match relative paths under `docs-site/docs/` without the `.md` suffix.

### `smelt docs show <topic>`

```
smelt docs show <topic>
```

Prints the markdown content of the named embedded documentation topic to stdout. The `.md` suffix is optional. If the topic is not found, the command prints an error with "did you mean?" suggestions and exits non-zero.

### `smelt docs path`

```
smelt docs path
```

Prints a message indicating that docs are embedded in the binary and advises using `docs list` and `docs show`. Does not print a filesystem path (there is none — docs are compiled into the binary).

### Markdown output structure

```
<output>/
├── index.md
└── models/
    ├── <model_name>.md
    └── ...
```

**`index.md`** contains:
- Project name (from `smelt.yml`) and model count
- Generation timestamp (ISO 8601 UTC)
- Models table: `Model | Materialization | Owner | Description` — model names link to `models/<name>.md`; rows in topological execution order
- Tag index: each tag with the list of linked model names

**`models/<name>.md`** contains:
- Model name as heading; model description (if set)
- Metadata: Materialization, Owner (if set), Tags (if any)
- Columns table: `Column | Type | Nullable | Description | Tests`
  - The per-column **Tests** cell lists the column-level test constraints declared under `columns.<col>.tests` in the model's frontmatter (`models.md` §"`columns:` — column metadata"). Cell content is the comma-separated test names; an empty cell means no column-level tests are declared.
- **Tests** section: a bulleted list of every test model that targets this model — i.e. every `materialization: test` model with `test.model: <this model>` in its frontmatter (`testing.md` §"`test:` frontmatter key"). Each bullet is the test name, linking to the test model's source location. The section is omitted when no test models target this one.
- Upstream dependencies (links to upstream model pages)
- Downstream dependencies (links to downstream model pages)
- Incremental config section (only if the model is incremental): granularity, partition column, event time column, unique key

### JSON catalog schema

`target/docs/catalog.json` (or `<output>/catalog.json`):

```json
{
  "project": {
    "name": "<string>",
    "model_count": <integer>,
    "generated_at": "<ISO8601 UTC>"
  },
  "models": {
    "<model_name>": {
      "name": "<string>",
      "description": "<string>",        // omitted if absent
      "owner": "<string>",              // omitted if absent
      "tags": ["<string>"],             // omitted if empty
      "materialization": "table" | "view" | "ephemeral" | "materialized_view" | "test",
      "path": "<absolute path>",
      "columns": [
        {
          "name": "<string>",
          "data_type": "<SQL type>",    // omitted if unknown
          "nullable": true | false,      // omitted if unknown
          "description": "<string>",     // omitted if absent
          "tests": ["<string>"],         // omitted if empty
          "expression": "<SQL expr>",
          "source": {
            "type": "computed" | "from_model" | "wildcard" | "external_table" | "unknown",
            "model": "<model_name>",    // present when type = from_model or wildcard
            "column": "<col_name>",     // present when type = from_model
            "table": "<table_name>"     // present when type = external_table
          }
        }
      ],
      "upstream": ["<model_name>"],     // direct dependencies only
      "downstream": ["<model_name>"],   // direct dependents only
      "incremental": {                  // omitted if not incremental
        "granularity": "<string>",
        "partition_column": "<string>",
        "event_time_column": "<string>",
        "unique_key": ["<string>"]      // omitted if empty
      }
    }
  },
  "execution_order": ["<model_name>"],
  "tag_index": {
    "<tag>": ["<model_name>"]
  }
}
```

- `models` keys are in alphabetical (BTreeMap) order.
- Fields with `null` or empty-list values are omitted from the JSON output (`skip_serializing_if`).
- Column `source.type` is a tagged enum with `"type"` as the discriminator.
- Column types and nullability come from the type inference system; if unavailable, the fields are omitted.

## Semantics

### Column description sources

Each column in the catalog is built from two sources, merged:

1. **Type inference** (Salsa DB): `data_type`, `nullable`, `expression`, and `source` (lineage). If type inference cannot determine a field, that field is omitted.
2. **Frontmatter metadata** (the `columns:` map in model YAML frontmatter): `description` and `tests`. The full `columns:` shape is owned by `models.md` §"`columns:` — column metadata" — this spec only specifies which keys are rendered into the catalog. If no frontmatter entry exists for a column, the description and tests fields are absent from the catalog output.

```yaml
columns:
  amount:
    description: "Order amount in USD"
    tests: [not_null, positive]
```

Columns present in the inferred schema but absent from frontmatter appear in the catalog without description or tests. Columns declared in frontmatter but absent from the inferred schema are not included in the catalog output.

### Embedded documentation

`smelt docs list`, `smelt docs show`, and `smelt docs path` operate on documentation embedded in the `smelt` binary at compile time via the `include_dir!` macro. The documentation is compiled from `docs-site/docs/` at build time. No filesystem access is needed at runtime; all operations read from static binary data.

Topic paths correspond to relative paths under `docs-site/docs/`, without the `.md` suffix. For example, the file `docs-site/docs/guide/seeds.md` is accessible as topic `guide/seeds`.

### Output directories

`smelt docs generate` creates `<output>/models/` if it does not exist. All output files are written fresh on each invocation — previous output is overwritten. No incremental output: the full catalog is regenerated each run.

### `--select` and test models

When `--select` is specified, only selected models appear in the output. Test models (`materialization: test`) are excluded from catalog output regardless of selector.

### Wide-reflection visibility

The wide-reflection accessors `smelt.models.with_tag`, `smelt.models.all`, `smelt.sources.with_tag`, and `smelt.sources.all` observe the same model and source identities that the catalog renders: workspace-relative `path`, short `name`, and merged `tags`. The `path` and `name` fields on `ModelRef` / `SourceRef` values are derived from the same entity resolution that produces catalog page paths and model names. The `tags` field on `ModelRef` is the same merged set (smelt.yml first, then frontmatter) that drives the catalog's tag index.

No normative extension to catalog rendering is introduced by the wide-reflection accessors. Deeper catalog changes — such as per-model cohort-of-origin fields or multi-source-of-origin column tracking — are deferred to later work tracked in `docs/plans/20260509-meta-language-overall.md`.

## Design

**Two formats serve different consumers.** Markdown is human-readable and suitable for static site deployment (e.g., GitHub Pages, Confluence). JSON is the integration format for programmatic consumers — data portals, lineage tools, BI catalog integrations. A single-format output was rejected: pure Markdown excludes programmatic consumers; pure JSON excludes human readers who want to browse catalog pages without writing a script.

**Column lineage from type inference.** The `source` field on each column (computed, from_model, wildcard, external_table, unknown) is derived from smelt's type inference system rather than explicit annotation. This means lineage is automatically correct for well-typed models and degrades gracefully (omitted or `unknown`) for models with incomplete type information. User-declared provenance maps were rejected: they drift when the SQL changes and require maintenance work that the type inferencer eliminates.

**Embedded docs in binary.** Embedding user documentation in the binary ensures that `smelt docs show` always returns docs that match the installed version. There is no risk of docs being installed separately or out of sync. The trade-off is that docs updates require a binary release.

**`upstream`/`downstream` are direct only.** The catalog records only direct dependencies and dependents, not the full transitive graph. This keeps the schema stable and avoids explosion for deep graphs; consumers can traverse the catalog recursively to compute transitive graphs.

## Constraints & Invariants

1. **Catalog is regenerated in full on each `smelt docs generate` run.** There is no incremental update; all output files are overwritten.
2. **Test models are excluded from catalog output.** `materialization: test` models do not appear in the generated catalog.
3. **JSON `models` keys are alphabetically ordered.** The `BTreeMap` serialization ensures stable, deterministic output.
4. **Embedded docs match the binary version.** `smelt docs show` returns documentation embedded at build time. It does not read from the filesystem.
5. **Column `source` is derived from type inference, not user-declared.** If the type inference system cannot determine lineage, `source.type` is `"unknown"`.

## Known Divergences / Open Questions

- **`--select` support undocumented.** The `--select` flag on `smelt docs generate` is implemented but not mentioned in the user guide.
- **No `--format` default documented.** The user guide does not state that `markdown` is the default format when `--format` is omitted.
- **Column tests are stored as strings, not validated.** The `tests` array in column frontmatter is stored as raw strings and surfaced in the catalog. There is no validation that test names correspond to actual test definitions.
- **`smelt docs path` is a stub.** The command prints a message rather than a usable path. This is intentional (docs are embedded) but the command's utility is unclear to users who expect a filesystem path.
- **Incremental section omitted for ephemeral models.** Ephemeral models that reference incremental config are not materialized; their incremental config section is not clearly handled in the catalog spec.
- **`generated_at` timezone.** The timestamp is UTC but the user guide does not specify this; the exact format string is not documented.

## References

- **Code**:
  - `crates/smelt-cli/src/commands/docs.rs` — `generate()`, `list()`, `show()`, `path()`, embedded docs via `include_dir!`
  - `crates/smelt-cli/src/docs.rs` — `build_catalog()`, `Catalog`, `CatalogModel`, `CatalogColumn`, `ColumnSource`
  - `crates/smelt-cli/src/docs_render.rs` — `render_markdown()`, `render_json()`
  - `crates/smelt-core/src/metadata.rs` — `ColumnMetadata` (description, tests, default, backfill)
- **User docs**:
  - `docs-site/docs/guide/data-catalog.md` (if present)
- **Related specs**:
  - `models.md` — frontmatter `columns:` key, `owner`, `tags`
  - `cli.md` — `smelt docs generate` command, `--select` flag
  - `model_selection.md` — selector syntax
