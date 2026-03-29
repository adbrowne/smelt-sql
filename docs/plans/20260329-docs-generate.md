# smelt docs generate — Data Catalog / Data Dictionary

**Date**: 2026-03-29
**Status**: Implemented

## Problem

smelt has rich model metadata (descriptions, owners, tags, column types with lineage, incremental config) but no way to export it as a data catalog. Users need a way to browse and share model documentation outside the LSP/UI.

## Solution

`smelt docs generate` produces a static data catalog in JSON or Markdown format.

### CLI Interface

```
smelt docs generate [--project-dir .] [--format markdown|json] [--output <dir>] [--select ...]
```

- **Markdown** (default): `index.md` + `models/<name>.md` per model
- **JSON**: Single `catalog.json` with full structured data

### What's Included

Per model:
- Name, description, owner, tags, materialization
- Columns: name, inferred type, nullability, frontmatter description, column-level tests
- Column lineage: source tracking (from_model, computed, wildcard, external_table)
- Upstream and downstream dependencies (deduplicated)
- Incremental config (granularity, partition/event-time columns, unique key)

Project-level:
- Model count, generation timestamp
- Execution order (topological)
- Tag index (tag → model list)

### Architecture

Follows the `explain` command pattern:

1. Discover models → build `LogicalGraph` → init Salsa DB
2. `build_catalog()` assembles a `Catalog` struct by merging Salsa type inference (column types, lineage) with frontmatter metadata (descriptions, tests)
3. Renderers serialize to the chosen format

### Key Files

- `crates/smelt-cli/src/docs.rs` — `Catalog` types + `build_catalog()` assembly
- `crates/smelt-cli/src/docs_render.rs` — JSON and Markdown renderers
- `crates/smelt-cli/src/commands/docs.rs` — CLI command handler
- `crates/smelt-cli/src/main.rs` — `Docs` command with nested `Generate` subcommand

### Deferred

- HTML output (use a static site generator on the JSON)
- `smelt docs serve` (nested subcommand structure supports this)
- Column lineage visualization
- `smelt docs diff`
