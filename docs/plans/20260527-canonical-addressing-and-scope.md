# Plan: Canonical `smelt.<path>` addressing + CLI `--scope` shorthand

**Date**: 2026-05-27
**Spec**: [`docs/specs/cli.md`](../specs/cli.md), [`docs/specs/architecture.md`](../specs/architecture.md), [`docs/specs/model_selection.md`](../specs/model_selection.md)
**Spec diff**:
  - `cli.md`: added §"Argument resolution and `--scope`" (Surface), the §"Argument resolution algorithm" + §"Cwd-derived scope computation" (Semantics), three Design paragraphs on scope-as-input, cwd-auto, and no-scope-is-hard-error, and Constraints 10–11 (canonical display, single resolver).
  - `architecture.md`: added Constraint & Invariant 9 (canonical-address invariant — `smelt.<path>` is the only resolution key in non-display layers).
  - `model_selection.md`: §"Selection methods" — ModelName selectors flow through CLI argument resolution; ambiguous bare leaves error.
**Tracking branch**: `feat/canonical-addressing-and-scope` (create from `main`).
**Docs**: spec already landed in this PR; `docs-site/` updates land in Phase 7.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/cli.md` §"Argument resolution and `--scope`" + §"Argument resolution algorithm" + §"Cwd-derived scope computation" (the correctness oracle for the CLI changes) and `docs/specs/architecture.md` §"Resolution" + Constraint & Invariant 9 (the canonical-address invariant).
2. Confirm you are on the tracking branch `feat/canonical-addressing-and-scope` off `main`.
3. Find the next `pending` phase in Progress tracking.

**For each phase:** red-green TDD on the listed tests, then commit and push using the phase's `Commit.` line verbatim. Real-fixture coverage: `examples/web_analytics/` (multi-layer: `bronze` / `silver` / `gold` / `marts`) is the load-bearing fixture for scope and canonical paths. `examples/functions_demo/` is the second-project fixture for the project-isolation invariant.

**Conventions every phase:**
- Red-green TDD: failing test before any implementation. The TDD tests listed in each phase are the *required* coverage; you may add more.
- Atomic per-phase commits using the phase's `Commit.` line verbatim.
- Run the standing gates before each commit: `cargo fmt --all`, `cargo clippy --all-targets`, `cargo test --quiet 2>&1 | tail -40`, `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces`.
- Never skip hooks, never `--no-verify`, never force-push.
- Honour `CLAUDE.md` invariants. The Pure Function Rule, Workspace Loading Parity Rule, Project Isolation Rule, and Run Pipeline Parity Rule all continue to apply unchanged.
- **Timeless-oracle rule.** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/` and `CLAUDE.md` describe the surface as if it has always existed.

---

## Context

The architecture spec's universal addressing scheme (`smelt.<path>`) is normative: every entity has a single canonical address that is the scan-root-stripped workspace-relative path. Today the implementation honours this in some layers and not others:

- `ModelFile.name` is the **leaf only** (file stem or `--- name: ---` value). `ModelFile.address_segments` carries the canonical path tuple.
- `smelt-db` has **two parallel resolvers** in `crates/smelt-db/src/lib.rs`: the legacy `resolve_ref(leaf)` (matches first model whose `model.name == leaf`) and the canonical `resolve_ref_path(path_tuple)` (strict path match). Both run in `file_diagnostics`. The legacy resolver silently accepts `smelt.events_parsed` when the actual model is `smelt.silver.events_parsed`; the path resolver emits an `UndefinedModelRef` diagnostic alongside. Net: confusing diagnostics + leaf-only refs that should be hard errors still resolve through some code paths.
- `DependencyGraph` (`crates/smelt-core/src/graph.rs`) is keyed on leaf-only `model.name`. Same-leaf collisions in different namespaces are "last writer wins" without a diagnostic.
- CLI `smelt type <name>` matches by `m.name == model_name` (leaf only). `smelt --select events_parsed` matches by leaf. There is no `--scope` flag.
- UI (`crates/smelt-ui/src/build.rs`, `server.rs`) labels nodes by leaf model name.

The spec changes in this PR add three rules:

1. **Inside model SQL**, every `smelt.<path>` reference is fully qualified. Leaf-only refs become a hard `UndefinedModelRef`.
2. **The CLI surface** accepts shorthand identifiers through `--scope` (or cwd-derived auto-scope), and always *prints* canonical paths. The shorthand expansion never recurses, never silently picks among ambiguous bare leaves.
3. **All non-display layers** (DependencyGraph, run manifest, selection engine, downstream consumers) key on the canonical dot-path string. Leaf-only names exist only as a parsed-out diagnostic field, never as a resolution key.

This plan closes the gap in seven phases. Phase order is chosen so each phase can land on its own:

- **Phase 1** introduces the canonical-path accessor without changing any keys — pure additive.
- **Phase 2** adds the strict-refs diagnostic (`UndefinedModelRef` on leaf-only) — diagnostic-only, no resolution path change. Verified against `examples/` which already use full paths.
- **Phase 3** rekeys `DependencyGraph` and every consumer — the invasive structural change.
- **Phase 4** introduces `--scope` and the argument-resolution algorithm — depends on Phase 1; CLI now accepts shorthand and prints canonical.
- **Phase 5** propagates canonical paths through the UI API and React surface.
- **Phase 6** deletes `resolve_ref(leaf)` and the `model_refs` leaf-extraction code path — structural enforcement of Invariant 9.
- **Phase 7** updates user docs (`docs-site/`) with `--scope` examples and canonical-path usage.

## Scope

### In scope

- New `canonical_path()` accessor on `ModelFile` returning the dot-joined `address_segments`.
- `DependencyGraph` (`crates/smelt-core/src/graph.rs`) keyed by canonical dot-path string. The `dependencies` and `models` HashMaps switch from `HashMap<String, _>` (leaf-keyed) to `HashMap<String, _>` (canonical-path-keyed). The `path_dependencies: HashMap<Vec<String>, _>` field is merged into the primary edge map.
- New CLI top-level flag `--scope <prefix>` plus cwd-derived auto-scope. The argument resolution algorithm in `cli.md` §"Argument resolution algorithm" lives in a new `crates/smelt-cli/src/argument_resolution.rs` module and is consumed by every command that takes an entity identifier (`type`, `table`, `status`, `history`, `explain`, `diff`, `run --select`, `build --select`, `backbuild --select`, `seed --select`, `docs generate --select`).
- `file_diagnostics` in `crates/smelt-db/src/lib.rs` calls only `resolve_ref_path`. Leaf-only refs produce `UndefinedModelRef` with a "did you mean '<full path>'?" hint when exactly one entity's leaf matches.
- Deletion of `resolve_ref` (leaf-only) and the `model_refs` Salsa query (leaf-extracting). Every caller migrates to `resolve_ref_path` + `model_path_refs`.
- UI API serialization (`crates/smelt-ui/src/types.rs`, `server.rs`, `build.rs`) emits canonical paths for every model identifier — graph node IDs, dependency lists, label fields. React side (`ui/src/`) renders canonical paths.
- User docs at `docs-site/docs/reference/cli.md` (new `--scope` section + canonical-path examples in command outputs) and any `docs-site/docs/guide/` pages whose worked examples use leaf-only model identifiers in `--select` or in `smelt type` calls.

### Explicitly deferred

- **UI scope filter (URL query param `?scope=silver`).** Spec-level Section 4 of the brainstorm marked this as follow-up. The UI's first cut just displays canonical paths everywhere — no client-side filtering.
- **LSP shorthand completions.** The LSP today completes against leaf model names; canonical-path completions are aligned with the new resolver but the completion UX (does typing `s` complete to `silver.events_parsed` or to a list of all `silver.*`?) needs its own design pass. Out of scope for this plan; LSP completions remain leaf-based until a follow-up.
- **Decoupling namespace from directory path** (`smelt.payments.balances` for a model at `models/teams/payments/marts/balances.sql`). Still tracked under `architecture.md` → Known Divergences. Out of scope.
- **Renaming `--select` to support glob/regex.** The selector grammar in `model_selection.md` stays as-is; only the resolution path of `ModelName` selectors changes.
- **Backwards-compat shims.** Project is pre-1.0 (CLAUDE.md "no backward compat constraints"). No deprecation cycle for the leaf-only resolver; it just goes away in Phase 6.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1 — Add `canonical_path()` accessor on `ModelFile`; populate `address_segments` everywhere it is constructed. | done | d2d0772f | 2026-05-27 |
| 2 — Strict full-path refs in model SQL: `file_diagnostics` calls only `resolve_ref_path`; leaf-only refs emit `UndefinedModelRef` with "did you mean" hint. | done | 38311551 | 2026-05-27 |
| 3 — Rekey `DependencyGraph` by canonical path. Update `smelt-runtime`, `smelt-cli`, `smelt-ui` callers. | done | 35fd2aec | 2026-05-27 |
| 4 — CLI `--scope` flag + argument-resolution module. Every entity-taking command resolves through it; output emits canonical paths. | pending |  |  |
| 5 — UI surface emits canonical paths in API and React. | pending |  |  |
| 6 — Delete legacy `resolve_ref(leaf)` and `model_refs` Salsa query. Update remaining callers (tests, test_harness) to canonical resolvers. | pending |  |  |
| 7 — User docs: `docs-site/docs/reference/cli.md` documents `--scope`; guide pages use canonical-path examples. | pending |  |  |

---

### Phase 1: `canonical_path()` accessor on `ModelFile`

**Goal.** Add a single source of truth for an entity's canonical dot-path string. No keys change yet; this is the foundation phase. Every subsequent phase consumes `canonical_path()` so the dot-join logic and the "what counts as canonical" decision live in one place.

**Pre-conditions.** None — entry point.

**TDD tests to write first.**

- `crates/smelt-core/src/discovery.rs` (inline `#[cfg(test)]`): `canonical_path_single_model_file` — given a `ModelFile` discovered from `models/silver/events_parsed.sql` under `paths: ["models"]`, `model.canonical_path()` returns `"silver.events_parsed"`.
- `crates/smelt-core/src/discovery.rs`: `canonical_path_multi_model_file` — given a multi-model file at `models/staging/pairs.sql` declaring `--- name: orders ---` and `--- name: customers ---`, the two emitted `ModelFile`s yield canonical paths `"staging.orders"` and `"staging.customers"`.
- `crates/smelt-core/src/discovery.rs`: `canonical_path_no_scan_root_match` — a model file discovered outside the project's `paths:` (e.g. under a future `functions/` scan path) yields the full relative path joined by `.` (no prefix stripped).
- `crates/smelt-core/src/discovery.rs`: `canonical_path_at_scan_root` — a model file directly under a scan root (`models/users.sql` under `paths: ["models"]`) yields `"users"`.

**Implementation shape.**
- Add to `crates/smelt-core/src/discovery.rs::ModelFile`:
  ```rust
  impl ModelFile {
      /// Canonical dot-joined `smelt.<path>` address of this model.
      /// Equals `self.address_segments.join(".")`.
      pub fn canonical_path(&self) -> String {
          self.address_segments.join(".")
      }
  }
  ```
- Audit every place a `ModelFile` is constructed:
  - `crates/smelt-core/src/discovery.rs::ModelDiscovery::discover_models` — already calls `compute_address_segments`. Verify it runs for both single and multi-model files.
  - `crates/smelt-core/src/discovery.rs::parse_sql_file` — currently sets `address_segments: Vec::new()`. Pull the segment computation up so the function returns models with `address_segments` populated when given a scan root, or have the single caller (`ModelDiscovery`) fill them in. Recommendation: thread `scan_root: &Path` through `parse_sql_file` so address_segments is always populated.
  - `crates/smelt-core/src/workspace.rs::load_workspace` — the shared loader. Same population check.
  - `crates/smelt-ui/src/build.rs` (around `crates/smelt-ui/src/build.rs:549`) and `crates/smelt-ui/src/server.rs:221` — both construct synthetic `ModelFile`s with `address_segments: Vec::new()`. For now, leave as `Vec::new()` (Phase 5 swaps these to compute the canonical path from the synthetic input). Add a `// TODO Phase 5` comment.
  - `crates/smelt-runtime/` — search for any `ModelFile { … }` literals and confirm they all populate `address_segments`.
- Do not yet change any key in `DependencyGraph` or any consumer. This phase is read-only at the consumer surface.

**Critical files.**
- `crates/smelt-core/src/discovery.rs` (add accessor; verify population paths).
- `crates/smelt-core/src/workspace.rs` (verify population).
- `crates/smelt-ui/src/build.rs:549`, `crates/smelt-ui/src/server.rs:221` (mark TODO).

**Docs touched.** None this phase.

**Review checklist.**
- [ ] All four new tests in `discovery.rs` pass.
- [ ] `cargo test --quiet 2>&1 | tail -40` clean.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` clean.
- [ ] `grep -rn "ModelFile {" crates/` shows every construction site either populates `address_segments` or carries a `// TODO Phase 5` comment with explanation.

**Commit.** `feat(core): add canonical_path() accessor on ModelFile`

---

### Phase 2: Strict full-path refs in model SQL

**Goal.** A `smelt.<path>` reference inside a model SQL body that does not exactly match a workspace entity's canonical path is a hard `UndefinedModelRef` diagnostic. When the failed ref's leaf segment matches exactly one entity by leaf, the diagnostic includes a `did you mean '<full path>'?` hint. This phase changes the diagnostic surface only — Phase 6 deletes the underlying legacy resolver.

**Pre-conditions.** Phase 1 complete (`canonical_path()` available).

**TDD tests to write first.**

- `crates/smelt-db/tests/strict_refs.rs::leaf_only_ref_is_undefined` — given a workspace with `models/silver/events_parsed.sql` (a model with `FROM smelt.events_parsed` against `models/silver/raw_events.sql` — i.e. wrong, should be `smelt.silver.events_parsed`), `file_diagnostics` returns exactly one `Diagnostic` with `code = UndefinedModelRef` and message containing `did you mean 'smelt.silver.events_parsed'?`.
- `crates/smelt-db/tests/strict_refs.rs::correct_full_path_resolves` — given the same workspace but the model uses `FROM smelt.silver.events_parsed`, no diagnostics.
- `crates/smelt-db/tests/strict_refs.rs::ambiguous_leaf_no_hint` — given a workspace with `models/silver/events.sql` and `models/bronze/events.sql`, a third model with `FROM smelt.events` emits `UndefinedModelRef` whose message lists both candidates: `did you mean one of 'smelt.silver.events', 'smelt.bronze.events'?`.
- `crates/smelt-db/tests/strict_refs.rs::zero_leaf_matches_plain_diagnostic` — `FROM smelt.nonexistent` in a workspace with no model named `nonexistent` emits `UndefinedModelRef` with no "did you mean" hint.
- `crates/smelt-cli/tests/example_diagnostics.rs` (already exists): rerun — confirms every model in `examples/web_analytics/`, `examples/functions_demo/`, etc. still passes (they already use full paths, so this is a regression check).

**Implementation shape.**
- In `crates/smelt-db/src/lib.rs::file_diagnostics`, around line 1180–1195, delete the legacy loop:
  ```rust
  let refs = model_refs(db, file);
  for ref_loc in refs.iter() {
      let resolved = project.and_then(|p| resolve_ref(db, workspace, p, ref_loc.name.clone()));
      if resolved.is_none() && !is_known_seed(db, workspace, &ref_loc.name) {
          DiagnosticAcc(Diagnostic { … }).accumulate(db);
      }
  }
  ```
  The `model_path_refs` loop already emits `UndefinedModelRef` for unresolved paths; the deletion makes the path-form check authoritative.
- Extend the `model_path_refs` loop's `None` arm in `file_diagnostics` (around line 1220–1245) to compute the "did you mean" hint:
  - When `resolve_ref_path` returns `None`, collect every `SourceFile` whose `parse_model(db, file).map(|m| m.name) == ref_loc.path.last()`.
  - If exactly one match: append ` did you mean '<canonical>'?` where `<canonical>` is the matched file's `file_path_tuple(...)` joined by `.`.
  - If two or more matches: append ` did you mean one of '<a>', '<b>'[, ...]?`.
  - If zero matches: no hint.
- Encapsulate the hint computation as a pure function `fn leaf_did_you_mean(workspace: Workspace, db: &dyn salsa::Database, leaf: &str) -> Vec<String>` (returns canonical paths). Tested in isolation with a constructed workspace in `crates/smelt-db/tests/strict_refs.rs::did_you_mean_helper_single_match` and `…multi_match`.

**Critical files.**
- `crates/smelt-db/src/lib.rs::file_diagnostics` (delete legacy loop, extend path-form error).
- `crates/smelt-db/src/lib.rs::leaf_did_you_mean` (new, near `resolve_ref_path`).
- `crates/smelt-db/tests/strict_refs.rs` (new).

**Docs touched.** None this phase (cli.md already describes the diagnostic surface).

**Review checklist.**
- [ ] All five new tests pass.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` clean.
- [ ] `cargo test -p smelt-lsp --test example_workspaces` clean.
- [ ] `cargo test --quiet 2>&1 | tail -40` clean.
- [ ] `model_refs(db, file)` is **not** removed in this phase (Phase 6); it must remain callable for tests that have not yet been migrated.

**Commit.** `feat(db): strict full-path resolution for smelt.<path> refs in model SQL`

---

### Phase 3: Rekey `DependencyGraph` by canonical path

**Goal.** `DependencyGraph` becomes the structural enforcement of Invariant 9 — its `dependencies` and `models` maps are keyed by the canonical dot-path string, not the leaf. The redundant `path_dependencies: HashMap<Vec<String>, …>` field is removed (merged into the primary map). Every consumer (`smelt-runtime`, `smelt-cli`, `smelt-ui`) updates to construct and consume `DependencyGraph` with canonical-path keys.

**Pre-conditions.** Phase 1 complete.

**TDD tests to write first.**

- `crates/smelt-core/src/graph.rs` (inline `#[cfg(test)]`): `same_leaf_distinct_canonical_paths_coexist` — a workspace with `models/silver/events.sql` and `models/bronze/events.sql` builds a `DependencyGraph` containing both keys (`"silver.events"` and `"bronze.events"`); `graph.get_model("silver.events")` and `graph.get_model("bronze.events")` both return Some, and they are distinct models.
- `crates/smelt-core/src/graph.rs`: `dependencies_use_canonical_paths` — a model at `models/gold/daily.sql` with `FROM smelt.silver.events_parsed` produces `graph.dependencies()["gold.daily"] == vec!["silver.events_parsed"]`.
- `crates/smelt-core/src/graph.rs`: `topological_order_uses_canonical_paths` — `graph.topological_order()` returns `Vec<String>` of canonical paths in DAG order.
- `crates/smelt-runtime/tests/select_parity.rs` (already exists): every assertion that compares against a leaf-name string must update to compare against the canonical path. Update the assertions and ensure the test stays green.
- `crates/smelt-runtime/tests/execute_parity.rs` (already exists per Run Pipeline Parity Rule): assert that `RunManifest` entries are keyed by canonical paths and that CLI ↔ UI agree on the same key set.

**Implementation shape.**

- In `crates/smelt-core/src/graph.rs`:
  - Rename the conceptual key throughout: `dependencies: HashMap<String, Vec<String>>` and `models: HashMap<String, ModelFile>` both use canonical dot-path strings. The variable names already say `model_name` — update to `model_path` for clarity in new code; existing `model_name` parameters can stay if a rename would explode the diff.
  - Delete `path_dependencies: HashMap<Vec<String>, Vec<Vec<String>>>` and its `build_from_workspace` plumbing. Every caller that used `Vec<String>` keys switches to the joined dot-path. (The `Vec<String>` form survives only inside `resolve_ref_path`, not in the graph.)
  - In `DependencyGraph::build`, replace `dependencies.insert(model.name.clone(), deps)` with `dependencies.insert(model.canonical_path(), deps)`. The `deps` Vec must be canonical paths too — compute each dep's canonical path by resolving its `RefInfo.smelt_ref.to_path()` against the workspace; for refs that fail to resolve, fall back to `path.join(".")` (these will already be flagged as `UndefinedModelRef` in Phase 2).
  - Update `validate_targets`, `target_assignments`, and `topological_order` to operate on canonical-path keys.

- In `crates/smelt-runtime/`:
  - Search for every `graph.get_model(<leaf>)` callsite. Update to canonical-path argument. The compile pipeline (`compile_model`, `expand_ephemerals`) walks `graph.dependencies()` and looks up each dep — these now hand back canonical paths automatically.
  - `RunManifest` entries: confirm the manifest key is the canonical path. If `crates/smelt-runtime/src/manifest.rs` uses leaf names, switch to canonical.
  - `RunReporter` trait methods that name a model: switch parameter from `&str` (leaf) to `&str` (canonical path). Trait surface only changes in field meaning; the type signature is unchanged.

- In `crates/smelt-cli/src/commands/run.rs` and adjacent:
  - The progress printer feeding stdout uses canonical paths. The CLI's reporter wrapper formats them.
  - Selection: convert selector strings to canonical paths (Phase 4 builds this), but in this phase the simpler change is: where the runtime fields a `Vec<String>` of selected model names, treat them as canonical paths.

- In `crates/smelt-ui/`:
  - `crates/smelt-ui/src/run_manager.rs`, `build.rs`, `server.rs`: every `model.name` reference for graph operations becomes `model.canonical_path()` (or the canonical key already constructed by the runtime). UI display strings (Phase 5) are a separate sweep; this phase just keeps the build green.

**Critical files.**
- `crates/smelt-core/src/graph.rs` (rekey).
- `crates/smelt-runtime/src/registry.rs`, `src/execute.rs`, `src/compile.rs` (search for `get_model` and graph iteration).
- `crates/smelt-runtime/src/manifest.rs` (if exists; otherwise the manifest module lives in `smelt-cli/src/manifest.rs` — same audit).
- `crates/smelt-cli/src/commands/run.rs`, `src/commands/build.rs`, `src/commands/backbuild.rs`.
- `crates/smelt-ui/src/run_manager.rs`, `src/build.rs`, `src/server.rs`.
- `crates/smelt-runtime/tests/select_parity.rs`, `tests/execute_parity.rs` (update assertions).

**Docs touched.** None this phase.

**Review checklist.**
- [ ] `same_leaf_distinct_canonical_paths_coexist` passes.
- [ ] `select_parity` and `execute_parity` tests green.
- [ ] `cargo test --quiet 2>&1 | tail -40` clean.
- [ ] `cargo test -p smelt-cli --test example_diagnostics`, `cargo test -p smelt-lsp --test example_workspaces` clean.
- [ ] `examples/web_analytics/` builds end-to-end via `cargo run -p smelt-cli -- --project-dir examples/web_analytics build` and produces the expected models.
- [ ] No `graph.dependencies` or `graph.models` callsite uses a leaf-only key.

**Commit.** `feat(core): rekey DependencyGraph by canonical smelt.<path>`

---

### Phase 4: CLI `--scope` flag + argument resolution

**Goal.** Implement `cli.md` §"Argument resolution algorithm" and §"Cwd-derived scope computation" as a reusable module consumed by every CLI command that takes an entity identifier. Add the top-level `--scope` flag. Every output prints canonical paths.

**Pre-conditions.** Phases 1 and 3 complete (canonical paths available everywhere; `DependencyGraph` already canonical-keyed).

**TDD tests to write first.**

- `crates/smelt-cli/src/argument_resolution.rs` (new, inline `#[cfg(test)]`):
  - `auto_scope_from_cwd_under_scan_root` — cwd `<project>/models/silver` with `paths: ["models"]` returns auto-scope `Some(vec!["silver"])`.
  - `auto_scope_from_cwd_deep_under_scan_root` — cwd `<project>/models/marts/daily` returns `Some(vec!["marts", "daily"])`.
  - `auto_scope_from_cwd_at_scan_root` — cwd `<project>/models` returns `None` (cwd is scan root itself).
  - `auto_scope_from_cwd_at_project_root` — cwd `<project>` returns `None`.
  - `auto_scope_from_cwd_outside_project` — cwd `/tmp` returns `None`.
  - `explicit_scope_overrides_cwd` — `--scope marts` with cwd inside `models/silver` returns `Some(vec!["marts"])`.
  - `empty_scope_disables_auto` — `--scope ""` with cwd inside `models/silver` returns `None`.
  - `scope_rejects_leading_smelt` — `--scope smelt.silver` is a hard config error (clap or a post-parse check).
  - `scope_rejects_whitespace` — `--scope " silver "` is a hard config error.
  - `resolve_arg_scoped_match_first` — workspace has `silver.events_parsed`; with scope `silver`, arg `events_parsed` resolves to `silver.events_parsed`.
  - `resolve_arg_falls_through_to_bare` — workspace has `silver.events_parsed` and `bronze.events_parsed`; with scope `gold`, arg `bronze.events_parsed` (a full path; the scoped expansion `gold.bronze.events_parsed` doesn't exist) resolves to `bronze.events_parsed`.
  - `resolve_arg_bare_leaf_no_scope_ambiguous_errors` — workspace has `silver.events` and `bronze.events`; with no scope, arg `events` errors with both candidates listed.
  - `resolve_arg_bare_leaf_no_scope_single_match_hints` — workspace has only `silver.events_parsed`; with no scope, arg `events_parsed` errors with `did you mean 'silver.events_parsed'?` (note: this is the no-scope hard-error rule from cli.md §Design).
  - `resolve_arg_full_path_always_works` — workspace has `silver.events_parsed`; with scope `marts`, arg `silver.events_parsed` resolves (first candidate `marts.silver.events_parsed` fails, second resolves).
- `crates/smelt-cli/tests/scope_integration.rs` (new):
  - `smelt_type_with_cwd_scope` — driven via `assert_cmd` against `examples/web_analytics/`. Set `current_dir` to `examples/web_analytics/models/silver`, run `smelt type events_parsed`. Stdout starts with `silver.events_parsed:`.
  - `smelt_type_with_explicit_scope` — `--scope silver` + arg `events_parsed` produces the same output regardless of cwd.
  - `smelt_type_canonical_arg` — no scope, `smelt type silver.events_parsed` produces the same output.
  - `smelt_type_no_scope_bare_leaf_errors` — no scope, `smelt type events_parsed`, asserts non-zero exit and stderr contains `did you mean 'silver.events_parsed'?`.
  - `smelt_type_output_uses_canonical_path` — `smelt type` with no positional arg lists models; assert every printed line starts with a canonical dot-path (`bronze.raw_events:`, `silver.events_parsed:`, etc.), no bare leaves.
  - `smelt_run_select_scope` — from cwd inside `examples/web_analytics/models/silver`, `smelt run --select events_parsed` selects `silver.events_parsed`.
  - `smelt_run_select_ambiguous_errors` — create a fixture in `examples/web_analytics/models/gold/events_parsed.sql` (temporarily, or use a fresh test fixture), confirm `smelt --scope "" run --select events_parsed` errors with both matches listed.

**Implementation shape.**

- New module `crates/smelt-cli/src/argument_resolution.rs`:
  ```rust
  pub struct Scope {
      pub segments: Vec<String>,
  }

  pub fn compute_scope(
      project_root: &Path,
      cwd: &Path,
      scan_roots: &[String],
      explicit: Option<&str>,
  ) -> Option<Scope> { ... }

  pub fn resolve_argument(
      db: &dyn salsa::Database,
      workspace: Workspace,
      scope: Option<&Scope>,
      arg: &str,
  ) -> Result<Vec<String>, ResolutionError>;
  ```
  - `compute_scope` honours the precedence in `cli.md` §"Argument resolution and `--scope`": explicit `Some(non-empty)` wins; `Some("")` returns None; absent → cwd-derived.
  - `resolve_argument` implements `cli.md` §"Argument resolution algorithm" 1–5 verbatim. Returns `Vec<String>` (canonical path segments) on success; `ResolutionError::NotFound { arg, hints: Vec<String> }` and `ResolutionError::Ambiguous { arg, matches: Vec<String> }` on failure.
  - The `did_you_mean` hint logic is the same as Phase 2's `leaf_did_you_mean`; extract that helper to `crates/smelt-db/src/lib.rs::pub fn leaf_did_you_mean` so both Phase 2's diagnostic and Phase 4's CLI errors share it.

- Add the top-level `--scope` flag in `crates/smelt-cli/src/main.rs` (the clap derive struct). Pass it through to every command that takes an entity identifier.

- Per-command updates (`crates/smelt-cli/src/commands/`):
  - `type.rs` — replace `models.iter().find(|m| m.name == *model_name)` with `resolve_argument(db, ws, scope.as_ref(), model_name)` and look up the resulting canonical path through the new graph API. Output already changed to canonical-path-keyed in Phase 3; just confirm.
  - `table.rs`, `status.rs`, `history.rs`, `explain.rs`, `diff.rs` — same treatment for any positional model arg.
  - `run.rs`, `build.rs`, `backbuild.rs`, `seed.rs` — `--select`/`--exclude` values pass through `resolve_argument` before being handed to the selection engine. Selectors with a `:` (e.g. `tag:revenue`, `generator_file:...`) pass through unchanged.
  - All printed output: confirm every model identifier emitted to stdout/stderr is the canonical path. Where a command currently does `println!("{}", model.name)`, switch to `println!("{}", model.canonical_path())`.

- `crates/smelt-cli/src/main.rs`: surface the cwd-derived scope on the first line of `--verbose` output, e.g. `scope: silver (auto from cwd)` or `scope: silver (--scope)` or `scope: <none>`.

**Critical files.**
- `crates/smelt-cli/src/argument_resolution.rs` (new).
- `crates/smelt-cli/src/main.rs` (clap `--scope`, top-level plumbing).
- `crates/smelt-cli/src/commands/*.rs` (all entity-taking commands).
- `crates/smelt-db/src/lib.rs::leaf_did_you_mean` (extracted helper).
- `crates/smelt-cli/tests/scope_integration.rs` (new).

**Docs touched.** None this phase (cli.md already specs it; user docs land in Phase 7).

**Review checklist.**
- [ ] All `argument_resolution` unit tests pass.
- [ ] All `scope_integration` integration tests pass.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` clean.
- [ ] `cargo test --quiet 2>&1 | tail -40` clean.
- [ ] `smelt --help` shows `--scope` in the top-level flag table.
- [ ] No CLI command prints a leaf-only model identifier; grep `crates/smelt-cli/src/` for `.name)` and confirm every match is either a hidden internal field or already swapped to `.canonical_path()`.

**Commit.** `feat(cli): --scope flag + canonical argument resolution`

---

### Phase 5: UI surface emits canonical paths

**Goal.** The UI's HTTP API and the React app both surface canonical `smelt.<path>` for every model identifier. Graph node IDs, label fields, dependency lists, and any URL fragment carrying a model identifier all use canonical paths.

**Pre-conditions.** Phase 3 complete (DependencyGraph already canonical-keyed in the runtime).

**TDD tests to write first.**

- `crates/smelt-ui/tests/api_canonical_paths.rs` (new):
  - `models_endpoint_emits_canonical_paths` — boot the UI server in test mode against `examples/web_analytics/`. GET `/api/models`. Assert the JSON contains `silver.events_parsed`, `bronze.raw_events`, etc. (canonical paths) as node IDs; no bare `events_parsed` leaf appears anywhere in the response.
  - `dependencies_are_canonical_paths` — Same endpoint. Assert each node's `refs` / `dependencies` field is a list of canonical paths.
  - `same_leaf_distinct_namespaces` — fixture with two `events.sql` (one under `silver`, one under `bronze`). Both appear as distinct nodes in the API response.

**Implementation shape.**

- `crates/smelt-ui/src/types.rs`: any field named `name`, `model_name`, `target`, or similar on `ModelInfo`, `EdgeInfo`, etc. that carries a model identifier — confirm it is `String` and document via doc-comment that it is a canonical path.
- `crates/smelt-ui/src/build.rs` and `src/server.rs`:
  - Replace every `model.name.clone()` with `model.canonical_path()` for fields that flow into the API response.
  - The synthetic `ModelFile { …, address_segments: Vec::new() }` constructions noted in Phase 1's TODO — compute `address_segments` from whatever the synthetic input is (a `name` field carrying a canonical path string can split on `.`; an `address_segments: Vec<String>` parameter is cleaner).
- `crates/smelt-ui/src/run_manager.rs`: the run-reporter wrapper feeds model identifiers into WebSocket messages and the HTTP `/api/runs/<id>` response. Same swap.
- React side (`ui/src/`):
  - Find every `model.name` reference. Replace with `model.canonical_path` (or whatever the field is now called in the API response).
  - Graph view (`ui/src/components/Graph.tsx` or similar): node labels render the canonical dot-path. Long paths may need ellipsis truncation; if so, full path goes in the tooltip.
  - URL routing: any route that takes a model identifier as a path segment (e.g. `/models/silver.events_parsed`) accepts dotted identifiers. If `react-router` complains about dots, encode as `/models/silver/events_parsed` (slash-separated) and document the URL grammar.

**Critical files.**
- `crates/smelt-ui/src/types.rs`, `src/server.rs`, `src/build.rs`, `src/run_manager.rs`.
- `crates/smelt-ui/tests/api_canonical_paths.rs` (new).
- `ui/src/components/Graph.tsx`, `ui/src/routes/*.tsx` (or whatever the React entry points are).

**Docs touched.** None this phase (UI is not user-spec documented; the API is internal).

**Review checklist.**
- [ ] All `api_canonical_paths` tests pass.
- [ ] `cd ui && npm run build` succeeds with no type errors.
- [ ] Manually drive the UI against `examples/web_analytics/`: every node label shows canonical paths; no leaf-only names.
- [ ] `same_leaf_distinct_namespaces` fixture renders both models as distinct nodes.

**Commit.** `feat(ui): canonical smelt.<path> identifiers in API and React surface`

---

### Phase 6: Delete legacy `resolve_ref(leaf)` and `model_refs`

**Goal.** Structural enforcement of Invariant 9. The leaf-only resolver and the leaf-extracting Salsa query become non-functions; the type system prevents reintroduction.

**Pre-conditions.** Phases 2–5 complete. Every caller of `resolve_ref(leaf)` and `model_refs` has been migrated.

**TDD tests to write first.**

- `crates/smelt-db/src/lib.rs` (inline `#[cfg(test)]`):
  - Delete any tests that exercise `resolve_ref(leaf)` directly. (If any remain, they were not migrated in Phase 2/4.)
- `crates/smelt-db/tests/no_leaf_resolver.rs` (new):
  - `compile_fails_if_resolve_ref_leaf_returns` — *not a runtime test;* a `compile_error!`-guarded module that fails to compile if `resolve_ref` exists. (Use `#[cfg(any(test, feature = "ban-legacy-resolver"))]` and `pub use` to gate at the API level — see Implementation shape.)
  - Realistically: a `grep -q "fn resolve_ref" crates/smelt-db/src/lib.rs` invocation in a build-time check, or just a code-review checklist item. The structural guarantee is "the function no longer exists, so no one can call it."

**Implementation shape.**

- Delete `pub fn resolve_ref(…)` (around `crates/smelt-db/src/lib.rs:431`).
- Delete the `model_refs` Salsa query (`crates/smelt-db/src/queries/parse.rs` — find via `grep -rn "fn model_refs"`).
- Delete `crates/smelt-db/src/test_harness.rs::resolve_ref` and `resolve_ref_in_project` (lines ~156–176).
- Delete the `model_name: String` field on `RefInfo` (`crates/smelt-core/src/refs.rs:45`). Every consumer of `RefInfo` now reads `smelt_ref` (the unified path tuple). Update all consumers.
- Delete the test in `crates/smelt-core/src/refs.rs::tests::test_extract_refs_path_form` (and adjacent) that assert on `model_name`; rewrite to assert on `smelt_ref.to_path()`.
- Re-run all standing gates. Any test that fails because it called the deleted leaf resolver had been missed in an earlier phase — update the test to use canonical paths.

**Critical files.**
- `crates/smelt-db/src/lib.rs` (delete `resolve_ref`).
- `crates/smelt-db/src/queries/parse.rs` (delete `model_refs`).
- `crates/smelt-db/src/test_harness.rs` (delete leaf methods).
- `crates/smelt-core/src/refs.rs` (delete `model_name` field; update consumers).
- Every caller surfaced by `grep -rn "resolve_ref\b" crates/ | grep -v resolve_ref_path` (should be zero after the deletion).

**Docs touched.** None this phase.

**Review checklist.**
- [ ] `grep -rn "fn resolve_ref\b" crates/` returns no results (only `resolve_ref_path` remains).
- [ ] `grep -rn "fn model_refs\b" crates/` returns no results.
- [ ] `grep -rn "\.model_name\b" crates/` returns only `model_refs`-unrelated matches (e.g. UI display fields that have their own `model_name` for unrelated reasons — verify each is canonical).
- [ ] `cargo test --quiet 2>&1 | tail -40` clean.
- [ ] `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces` clean.
- [ ] `cargo clippy --all-targets` clean.

**Commit.** `refactor(db): delete legacy leaf-only resolver and model_refs query`

---

### Phase 7: User docs (`docs-site/`)

**Goal.** Update `docs-site/docs/reference/cli.md` to document `--scope` and the argument-resolution rules. Update any guide pages whose worked examples use leaf-only model identifiers in `--select` or `smelt type` commands.

**Pre-conditions.** Phase 4 complete (CLI flag and behavior are live).

**TDD tests to write first.** Docs phase — no code tests. The verification is reading the rendered docs site locally and checking the worked examples run.

**Implementation shape.**

- `docs-site/docs/reference/cli.md`:
  - Add a new top-level section after the existing "Common flags" subsection titled "Argument resolution and `--scope`" that mirrors the spec text but in user-doc voice (more examples, fewer normative references).
  - Walk every command's reference example: where the example shows `smelt type users` or `smelt run --select users`, add a sibling example showing the canonical-path form (`smelt type silver.users`) and the scope-shorthand form. Clarify that all `smelt` output is the canonical path.
- `docs-site/docs/guide/`:
  - `model-selection.md` (if exists): add a paragraph noting that selectors flow through scope expansion and unresolvable selectors with multiple matches are now an error.
  - Any walkthrough that uses leaf-only model identifiers in command examples should be reviewed; if the corresponding example workspace uses single-layer models (no namespacing), the existing examples are fine — the new shorthand subsumes them.
- Find unintentional leaf-only refs in docs-site code samples via `grep -rn "smelt\\.\\w\\+\\b" docs-site/docs/ | grep -v "smelt\\.\\w\\+\\."`. Audit each match — most should be deliberate single-segment paths (a model directly under `models/`, e.g. `smelt.users`); flag any that are actually leaf-truncated multi-segment paths and fix them.

**Critical files.**
- `docs-site/docs/reference/cli.md`.
- `docs-site/docs/guide/model-selection.md` (if present).
- Any docs-site page identified by the grep above.

**Docs touched.** All in `docs-site/`.

**Review checklist.**
- [ ] `cd docs-site && mkdocs serve` renders cleanly with no broken links.
- [ ] `cli.md` reference page documents `--scope` with at least three examples (cwd-derived, explicit, full-path).
- [ ] Every CLI command example that takes a model identifier shows the canonical path in output.
- [ ] No docs-site code sample uses a leaf-only path where the corresponding example workspace has a multi-layer model.

**Commit.** `docs(cli): --scope, canonical paths in worked examples`

---

## References

### Code (entry points)

- `crates/smelt-core/src/discovery.rs::ModelFile`, `compute_address_segments` — already computes the canonical path tuple; Phase 1 adds the accessor.
- `crates/smelt-core/src/graph.rs::DependencyGraph` — Phase 3 rekey target.
- `crates/smelt-db/src/lib.rs::resolve_ref`, `resolve_ref_path`, `file_diagnostics` — Phases 2 and 6.
- `crates/smelt-cli/src/commands/type.rs`, `src/commands/run.rs`, `src/commands/build.rs`, `src/main.rs` — Phase 4.
- `crates/smelt-ui/src/types.rs`, `src/server.rs`, `src/build.rs`, `ui/src/` — Phase 5.

### Tests

- `crates/smelt-cli/tests/example_diagnostics.rs` (standing gate; remains green).
- `crates/smelt-lsp/tests/example_workspaces.rs` (standing gate; remains green).
- `crates/smelt-runtime/tests/select_parity.rs`, `tests/execute_parity.rs` (rekey-aware after Phase 3).
- `crates/smelt-db/tests/strict_refs.rs` (new in Phase 2).
- `crates/smelt-cli/tests/scope_integration.rs` (new in Phase 4).
- `crates/smelt-ui/tests/api_canonical_paths.rs` (new in Phase 5).

### Related specs

- `docs/specs/cli.md` — the primary surface this plan implements.
- `docs/specs/architecture.md` §"Resolution" + Invariant 9 — the correctness oracle.
- `docs/specs/model_selection.md` §"Selection methods" — selector resolution rule.
- `docs/specs/scoping.md` — name-resolution rules *inside* `smelt.define` bodies; unchanged by this plan (scope is a CLI concept, never a body-resolution concept).

### Plans (history)

- `docs/plans/20260524-cli-runtime-migration.md` — established the Run Pipeline Parity Rule that this plan honours (no compile/execute logic moves into consumers; all goes through `smelt-runtime`).
