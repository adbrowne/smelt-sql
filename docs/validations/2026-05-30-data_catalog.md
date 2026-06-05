## Drift Report: data_catalog

**Spec**: docs/specs/data_catalog.md (last_reviewed: 2026-05-05)
**Date**: 2026-06-05

### Automated checks
- cargo fmt — PASS
- cargo clippy — PASS (0 warnings)
- cargo test — PASS
- example_diagnostics — PASS (94 passed, 1 ignored)

### Surface drift

- ✅ `smelt docs generate [--format markdown|json] [--output <dir>] [--select <selector>]` — implemented at `crates/smelt-cli/src/commands/docs.rs:88`
- ✅ `smelt docs list` — implemented at `crates/smelt-cli/src/commands/docs.rs:41`, sorted output
- ✅ `smelt docs show <topic>` — implemented at `crates/smelt-cli/src/commands/docs.rs:48`, with "did you mean?" on unknown topic and non-zero exit
- ✅ `smelt docs path` — implemented at `crates/smelt-cli/src/commands/docs.rs:80`, message format matches spec
- ✅ Markdown `index.md` — project name, model count, generation timestamp, models table in topological order, tag index — `docs_render.rs:59-96`
- ✅ Markdown `models/<name>.md` — model name, description, metadata block (materialization/owner/tags), columns table, upstream/downstream — `docs_render.rs:99-195`
- ❌ **Missing "Tests" section in Markdown model pages** — spec §Surface says each `models/<name>.md` must contain a "Tests" section (a bulleted list of every `materialization: test` model with `test.model: <this model>` in its frontmatter). `render_model_page()` (`docs_render.rs`) has no such section. `build_catalog()` (`docs.rs:89`) receives a graph from which test models have already been filtered (`commands/docs.rs:124` calls `models.retain(|m| !m.is_test())`), so the targeting info is never collected. Both Markdown and JSON output are missing this data. **BUG-048**
- ✅ JSON catalog schema — `Catalog`/`CatalogModel`/`CatalogColumn`/`CatalogColumnSource` structs match spec shape (`docs.rs:12-86`)
- ✅ `models` keys in `BTreeMap` order — alphabetical per spec Constraint 3
- ✅ `skip_serializing_if` for null/empty fields — all optional fields annotated
- ✅ Column `source` tagged enum with `"type"` discriminator — `#[serde(tag = "type")]` at `docs.rs:65`
- ✅ `origin` field for generator-emitted models, omitted for hand-authored — `docs.rs:242-248`
- ⚠️ KD stale: `--select support undocumented` — the CLI reference at `docs-site/docs/reference/cli.md:498` now documents the `--select` flag. Known Divergence should be removed.
- ⚠️ KD stale: `--format default not documented` — the CLI reference at `docs-site/docs/reference/cli.md:495` now documents `--format` with default `markdown`. Known Divergence should be removed.

### Semantics drift

- ✅ Column description sources (type inference + frontmatter merge) — `build_catalog()` at `docs.rs:115-187` merges schema columns with frontmatter metadata
- ✅ Embedded docs in binary — `include_dir!("$CARGO_MANIFEST_DIR/../../docs-site/docs")` at `commands/docs.rs:13`
- ✅ Topics accessible without `.md` suffix — `lookup_topic()` strips suffix before lookup
- ✅ Output directories created fresh each run — `fs::create_dir_all` in both render functions; no incremental output
- ✅ Test models excluded from catalog — `models.retain(|m| !m.is_test())` at `commands/docs.rs:124`
- ✅ `--select` filters models before catalog build — `commands/docs.rs:166-191`
- ✅ Generator-emitted model provenance rendered — `origin` field collected + rendered in both Markdown and JSON
- ⚠️ **Wide-reflection visibility semantics not test-covered** — spec §Semantics says `smelt.models.with_tag`/`smelt.models.all` etc. observe the same identities (path, name, tags) that the catalog renders. No test exercises this cross-feature property.
- ✅ Column `source` derived from type inference — `smelt_db::typed_model_schema` called for each node
- ❌ **"Tests" section — no test coverage** — no existing test asserts the Tests section appears in markdown output. The spec rule has zero coverage. **See BUG-048**.

### Invariant drift

- ✅ Constraint 1 (full regen per run) — `render_markdown`/`render_json` always overwrite; no read-before-write
- ✅ Constraint 2 (test models excluded) — `models.retain(|m| !m.is_test())` verified
- ✅ Constraint 3 (JSON alphabetical order) — `BTreeMap<String, CatalogModel>` used throughout
- ✅ Constraint 4 (embedded docs match binary) — `include_dir!` macro embeds at compile time
- ✅ Constraint 5 (column source from type inference) — `smelt_db::typed_model_schema` call path

### Timeless-oracle drift

- ✅ No phase-vocabulary leakage in spec body or user docs — grep returns nothing
- ⚠️ Test file comments reference "Phase 5 (meta-language-E2)" in `docs_json_output.rs:1` and `docs_markdown_output.rs:1` — these are test files (not spec/user docs), so not a timeless-oracle violation, but the pattern is worth noting.

### Freshness
- last_reviewed: 2026-05-05
- most recent code change: 2026-06-04 at `crates/smelt-cli/src/commands/docs.rs` (frontmatter-parity fixtures)
- Verdict: **stale** — spec last reviewed before the frontmatter-parity work landed. Recommend refreshing `last_reviewed` date once BUG-048 is fixed.

### Summary

- Drift items: 3 total — 1 surface (BUG-048: missing Tests section), 0 semantics code bugs (BUG-048 covers both surface and semantics gap), 2 stale KD entries (docs-gap, 1-line fixes)
- **BUG-048** is a clear code bug: code clearly violates spec Surface rule; fix is self-contained (no architectural invariant touched).
- Recommended next step: fix BUG-048 (red-green), then remove the two stale KD entries from the spec.
