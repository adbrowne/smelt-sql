## Drift Report: architecture (D7 probe)

**Spec**: docs/specs/architecture.md (last_reviewed: 2026-06-04)
**Date**: 2026-06-08

### Automated checks
- cargo fmt — PASS
- cargo clippy — PASS (zero warnings)
- cargo test — PASS
- example_diagnostics — PASS (101 passed, 1 ignored)
- example_workspaces — PASS (32 passed, 0 failed)

### Surface drift
- ✅ Crate table (smelt-types, smelt-parser, smelt-core, smelt-logical, smelt-db, smelt-dialect, smelt-planner, smelt-lsp, smelt-cli, smelt-backend, smelt-backend-duckdb, smelt-runtime, smelt-ui) — all crates present in workspace
- ✅ `smelt-logical` crate: sits above smelt-core/parser/types, below both smelt-db and smelt-planner — confirmed by `cargo tree -p smelt-db -i smelt-planner` returning "no packages" (no production path; BUG-064 fixed 2026-06-08)
- ✅ `smelt.<path>` universal addressing scheme — enforced by `project_address_collisions` Salsa query
- ✅ Unified frontmatter rule — shared parser in smelt-core, deny_unknown_fields enforced; frontmatter-parity sub-plan done 2026-06-04
- ✅ Two-layer multi-model file format (`--- name: --- ` Layer 1, `---/---` Layer 2) — in smelt-parser/smelt-core
- ✅ Generator files (`generates: models`) — in meta_language, frontmatter detection by frontmatter field (BUG-066 fixed 2026-06-08)
- ✅ Default materialization name mapping — enforced by `db_name()` in smelt-core

### Semantics drift — Architectural invariants

#### Salsa purity rule
- ✅ Pure functions in type_inference.rs, schema.rs — no Salsa imports
- ✅ Thin orchestration wrappers: `file_diagnostics`, `type_context` — confirmed

#### Workspace loading parity rule
- ✅ Single path: `smelt_core::workspace::load_workspace` called by both CLI `init_db` and LSP `Backend::initialize`
- ✅ Standing CI gate: `cargo test -p smelt-lsp --test example_workspaces` — 32 tests pass

#### Project isolation rule
- ✅ `resolve_function` is project-scoped: `sig_lookup` uses `ProjectInput` — confirmed
- ✅ `resolve_ref_path` is project-scoped: iterates `workspace.projects(db)` — confirmed
- ✅ Standing CI gate `project_isolation_in_multi_project_workspace` — passes (sessionize cross-project)
- ✅ Standing CI gate `no_cross_project_column_type_leak_through_resolve_ref` — passes
- ✅ `DuplicateAddress` is project-scoped: `project_address_collisions` keyed on `ProjectInput` — confirmed
- ⚠️ **Test gap (now fixed)**: No explicit test that DuplicateAddress is never emitted cross-project. Added `no_duplicate_address_across_projects_in_multi_project_workspace` in example_workspaces.rs.
- ⚠️ **Test gap (now fixed)**: No explicit test that goto-def stays within project boundaries. Added `goto_def_on_function_call_stays_in_same_project` in example_workspaces.rs.

#### Run pipeline parity rule
- ✅ Both CLI (`commands/run.rs`) and UI (`run_manager.rs`) consume `execute_project` — confirmed
- ✅ `smelt-runtime` internals pub(crate): SqlCompiler constructors, PrintContext builders, compile_with_sql — verified by `surface_audit` test in smelt-runtime
- ✅ Standing CI gate: `cargo test -p smelt-runtime --test execute_parity`

#### Diagnostic parity rule
- ✅ Pre-execution gate runs `file_diagnostics` over Error-severity diagnostics — in gate.rs
- ✅ Diagnostic range encoding: TextRange internally, boundary conversion at emission points

#### Layered single-ownership
- ✅ `cargo tree -p smelt-db -i smelt-planner` shows no production path (output: "error: no packages matched" = correct; smelt-planner not reachable from smelt-db)

### Timeless-oracle drift
- ✅ No `Phase [A-Z0-9]+` matches in architecture.md spec body or Known Divergences

### Freshness
- last_reviewed: 2026-06-04
- Most recent code change touching reference paths: 2026-06-08 (BUG-064 smelt-logical extraction, BUG-066 generator detection, BUG-074 CLI-runtime migration)
- Verdict: **fresh** — spec was last reviewed 2026-06-04, post major refactors; last_reviewed should be updated to 2026-06-08

### Summary
- Drift items: 2 (both test-coverage gaps, both fixed by adding tests this iteration)
- Code bugs: 0
- Recommended next step: none (test coverage added; no spec drift)
