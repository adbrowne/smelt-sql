## Drift Report: D6 — model_selection × generators × cli

**Spec**: docs/specs/model_selection.md (last_reviewed: 2026-05-27), docs/specs/cli.md
**Date**: 2026-06-08

### Automated checks (pre-probe baseline)
- cargo fmt — PASS
- cargo clippy — PASS (zero warnings)
- cargo test — PASS
- example_diagnostics — PASS (101 passed, 1 ignored)
- smelt-runtime — PASS (8 passed)

### Seam focus

D6 probes the intersection of model-selection, generator-emitted models, and the CLI
`--select`/`--exclude` execution path. The key surface: `generator_file:path` selectors
that are supposed to select all emitted survivors from a named `.gen.sql` file.

### Findings

#### BUG-074: `generator_file:` selector silently matches nothing (FIXED)

**Found via**: code inspection + red-green test in `select_parity.rs`

**Symptom**: `smelt run --select generator_file:models/cohorts.gen.sql` returns exit 0
but executes zero models. Same for `--exclude`.

**Root cause**: `DependencyGraph::select_models` in `crates/smelt-core/src/graph.rs:308`
returned `vec![]` for `SelectionMethod::GeneratorFile`, with a comment saying callers
should use a `smelt-db resolve_generator_file_selector` helper. That helper does not
exist as a production function — only as inline test code in `project.rs`. The old
`smelt-cli/src/logical_graph.rs` had a working implementation that matched on a
`generator_file` field stored per-node. When the CLI-runtime migration
(`docs/plans/20260524-cli-runtime-migration.md`, done 2026-06-07) replaced
`logical_graph.rs` with `smelt-core/src/graph.rs`, this case was left unimplemented.

**Fix**: Implemented `GeneratorFile` matching directly in `DependencyGraph::select_models`
by checking each model's virtual path. Emitted models have virtual paths of the form
`<gen-dir>/<gen-filename>::<smelt-name>` (produced by `model_file_from_emitted_def`
in `smelt-cli/src/discovery.rs:72-80`). The fix:
1. Splits the path at `::` to extract the generator file path prefix
2. Checks if the generator file path (normalised to forward slashes) ends with the
   selector's workspace-relative path
3. Hand-authored models have no `::` in their paths and are not matched

**Regression tests added**: 3 tests in `crates/smelt-runtime/tests/select_parity.rs`:
- `test_generator_file_selector_matches_emitted_models` — 3 emitted models selected, hand-authored not
- `test_generator_file_selector_non_generator_matches_nothing` — no error on unresolvable path
- `test_generator_file_selector_with_upstream_expansion` — `+generator_file:` includes upstreams

### Tag selection on emitted models

Verified that `tag:X` selectors work correctly for emitted models:
- `EmittedModelDef.tags` (in `project.rs:499`) is "Tags from `ModelDef.tags` field,
  merged with frontmatter tags" — both are populated by the generator pipeline.
- The `model_file_from_emitted_def` sets `metadata.tags = emitted.tags` (discovery.rs:56).
- `DependencyGraph::select_models` for `Tag` uses `config.get_tags(name, metadata)` which
  merges smelt.yml tags + metadata tags — correct.
- The per_cohort_union example's `tags: [cohort]` frontmatter propagates to emitted models
  (verified at `project.rs:706` — frontmatter_tags prepended to per-ModelDef tags).
- **No bug**: tag selection works for emitted models.

### CLI surface alignment

`--select` and `--exclude` reach `select_executable_models` via `SelectionRequest` built
in `execute.rs:112-116`. Both are correctly plumbed. No drift found for this seam.

### Verified gates (post-fix)
- cargo fmt — PASS
- cargo clippy — PASS (zero warnings)
- cargo test — PASS
- example_diagnostics — PASS (101 passed, 1 ignored)
- smelt-runtime select_parity — PASS (8 tests, including 3 new generator_file tests)

### Summary

- 1 code bug found and fixed: BUG-074 (`generator_file:` selector broken post-migration)
- 3 regression tests added
- No spec drift (spec is correct; code was wrong post-migration)
- Tag selection on emitted models is correct — no action needed
