## Drift Report: model_selection

**Spec**: docs/specs/model_selection.md (last_reviewed: 2026-05-27)
**Date**: 2026-06-05

### Automated checks
- cargo fmt — PASS
- cargo clippy — PASS (zero warnings)
- cargo test — PASS
- example_diagnostics — PASS (94 passed, 1 ignored)
- example_workspaces — PASS (28 passed)

### Surface drift

- ✅ `model_name` selector — `SelectionMethod::ModelName` in `crates/smelt-core/src/selector.rs:30`
- ✅ `tag:X` selector — `SelectionMethod::Tag` in `selector.rs:32`
- ✅ `generator_file:path/to/file.gen.sql` selector — `SelectionMethod::GeneratorFile` in `selector.rs:34-38`
- ✅ `+model_name`, `model_name+`, `+model_name+` prefix/suffix modifiers — parsed via `strip_prefix`/`strip_suffix` in `parse_selector()` at `selector.rs:78-113`
- ✅ `--select` (`-s`) on `run` — `RunArgs.select: Vec<String>` at `main.rs:111-112`
- ✅ `--select` (`-s`) on `build` — `BuildArgs.select: Vec<String>` at `main.rs:292-293`
- ✅ `--select` (`-s`) on `diff` — `DiffArgs.select: Vec<String>` at `main.rs:394-395`
- ✅ `--select` (`-s`) on `explain` — `ExplainArgs.select: Vec<String>` at `main.rs:364-365`
- ✅ `--select` (`-s`) on `docs generate` — `DocsGenerateArgs.select: Vec<String>` at `main.rs:383-384`
- ✅ `--select` (`-s`) on `seed` — `SeedArgs.select: Vec<String>` at `main.rs:253-254`
- ❌ `--select` (`-s`) on `backbuild` — spec flags table says `backbuild` has `--select`, but `BackbuildArgs` at `main.rs:154` uses a positional `selector: String` arg. **BUG-046 (needs-review).**
- ✅ `--exclude` (`-e`) on `run`, `build`, `diff`, `explain` — all four have `exclude: Vec<String>` args
- ✅ Error on empty selector — `SelectorParseError::Empty` at `selector.rs:83`
- ✅ Error on `tag:` with no name — `SelectorParseError::EmptyTag` at `selector.rs:117`
- ✅ Error on `generator_file:` with no path — `SelectorParseError::EmptyPath` at `selector.rs:122`
- ✅ Error on `+` in non-prefix/suffix position — `SelectorParseError::InvalidCharacter('+')` at `selector.rs:112`
- ⚠️ User docs (`docs-site/docs/guide/model-selection.md`) reference `path:...` as a selector type — not a spec-defined method. **BUG-047 (docs-gap, fixed in this phase).**
- ⚠️ User docs only mention `run`, `build`, `explain` in the intro sentence for `--select`/`--exclude` availability; `diff`, `docs generate`, `seed` are omitted. Minor docs gap (deferred — fuller docs overhaul out of scope for one-line fix).

### Semantics drift

- ✅ Union of selectors — `select_models` builds a union by inserting into a `HashSet` across all selectors (`logical_graph.rs:432-487`)
- ✅ Exclusion is post-selection — `exclude_models` calls `select_models` on excludes then `difference` (`logical_graph.rs:489-497`)
- ✅ No `--select` = all models — `run.rs:236` checks `if resolved_select.is_empty()` and skips `select_models`
- ✅ Upstream traversal (`+prefix`) — `collect_upstream` recursive DFS via `node.dependencies` (`logical_graph.rs:571-579`)
- ✅ Downstream traversal (`+suffix`) — `collect_downstream` via `build_dependents_map` (`logical_graph.rs:596-614`)
- ✅ Tag matching: `node.tags.contains(tag)` in `select_models` at `logical_graph.rs:447-451`
- ✅ No-match is not an error — `select_models` returns `Ok(empty)` when nothing matches
- ✅ `ModelName` ambiguity error — `resolve_selector_args` → `resolve_argument` → `Err(ResolutionError::Ambiguous{…})` at `argument_resolution.rs:219-225`; wired in `run.rs:219`, `explain.rs:98`, `diff.rs:96`
- ✅ `GeneratorFile` selector matches nodes with matching `generator_file` field — `logical_graph.rs:454-463`
- ❌ **Seeds in upstream traversal** — `collect_upstream` only follows `self.nodes`; seeds in `self.seeds` were silently skipped. **BUG-045 (fixed in this phase).**
- ✅ Topological execution order — `filtered_execution_order` at `logical_graph.rs:419-426`
- ✅ Ephemeral models included in traversal — ephemeral models are in `self.nodes` like any other model; they're traversed correctly even though not materialized

### Invariant drift

- ✅ Invariant 1 (Selector syntax strict) — parser enforces this with `InvalidCharacter('+')` for any `+` not at prefix/suffix
- ✅ Invariant 2 (Union, not intersection) — `HashSet` accumulation confirms union semantics
- ✅ Invariant 3 (Exclusion after inclusions) — `exclude_models` is always called after `select_models` in all command paths
- ✅ Invariant 4 (No-match not an error) — `Ok(empty)` returned, verified by test `select_no_match_tag_returns_empty_ok`
- ⚠️ Invariant 5 (Tag case-sensitive) — tag matching uses `node.tags.contains(tag)` which is Rust `String` equality (case-sensitive). No explicit test asserts `tag:Revenue` ≠ `tag:revenue`. Not a drift item — behavior is correct per spec, just untested.
- ✅ Invariant 6 (`GeneratorFile` matches post-resolution workspace shape) — `annotate_emitted_models` is called after `build()` and only surviving emitted models get `generator_file` set (`logical_graph.rs:256-270`)

### Test coverage gaps (spec rules with no test)

These are not code bugs but are unfiled test coverage gaps that the sweep adds coverage for in this phase:

- ✅ Downstream traversal — added `select_downstream_includes_dependents` test
- ✅ Union of multiple selectors — added `select_multiple_selectors_union` test
- ✅ Exclusion — added `exclude_models_removes_from_selection` test
- ✅ No-match empty set — added `select_no_match_tag_returns_empty_ok` test
- ✅ `GeneratorFile` in graph traversal — added `select_generator_file_matches_emitted_models` test
- ✅ Seeds in upstream traversal — added `select_upstream_includes_seed_dependencies` test (BUG-045 regression test)

### Timeless-oracle drift

- ✅ No phase-vocabulary leakage in spec body or user docs
- ⚠️ `crates/smelt-core/src/selector.rs:282` has `// ── Phase 5 (E2): generator_file: selector tests` — phase vocab in an implementation file test comment. Not a timeless-oracle violation (rule applies to spec body and user docs only), but informally bad practice. Not blocking.

### Freshness

- last_reviewed: 2026-05-27
- Most recent code change to `selector.rs`/`logical_graph.rs`: 2026-06-02 (8fc33c4a — wide reflection compile-time evaluation)
- Verdict: spec is current; code changes since last_reviewed were additive and do not conflict with spec rules

### Summary

- Drift items: 3 total (1 surface/backbuild, 1 code/seeds-upstream, 1 docs/path-selector)
- Fixed in this phase: BUG-045 (seed upstream traversal), BUG-047 (docs path: reference)
- Needs-review: BUG-046 (backbuild positional vs --select)
- Added 6 new test cases covering previously-untested spec rules
- Recommended next step: resolve BUG-046 in post-sweep human review (amend spec to document backbuild's positional selector interface)
