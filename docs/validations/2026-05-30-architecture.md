# Drift Report: architecture

**Spec**: docs/specs/architecture.md (last_reviewed: 2026-05-05)
**Date**: 2026-05-30
**Probe phase**: A1 (feature-sweep)

### Automated checks
- cargo fmt — PASS
- cargo clippy --all-targets — PASS (no warnings)
- cargo test — PASS (baseline: 171 test groups, 0 failures)
- example_diagnostics — PASS (76 passed, 1 ignored)
- example_workspaces (project-isolation + workspace-loading parity gate) — PASS (21 passed)
- smelt-runtime parity (compile_parity + select_parity) — PASS (5 passed)

### Surface drift
- ✅ `smelt.<path>` universal addressing — `resolve_ref_path` present; canonical-address invariant gated by example_diagnostics/example_workspaces.
- ✅ Compilation pipeline crates and `execute_project` entry point — `crates/smelt-runtime/src/execute.rs:103`.
- ✅ Workspace-loading parity — single `load_workspace` (`crates/smelt-core/src/workspace.rs:78`) + `ingest_loaded_workspace` (`crates/smelt-db/src/workspace_ingest.rs:43`); consumed by CLI + LSP; gated.
- ✅ Project isolation — `resolve_function`/`resolve_function_path` project-scoped (`crates/smelt-db/src/queries/functions.rs:137,178`); multi-project case in example_workspaces.
- ❌ **Address-uniqueness / "one path → one entity" (Surface §Resolution lines 103,105; Constraint #9) is unenforced.** A workspace with `models/dup.sql` (bare SELECT) + `models/dup.csv` (seed) — same dir, same stem — must be a hard workspace-load error. Observed: `smelt explain` exits 0, silently surfaces only the `.sql` model, drops the `.csv`. No collision check exists in `crates/smelt-core/src/discovery.rs` or `workspace.rs`. → **BUG-002**.

### Semantics drift
- ✅ Stage rules, identity properties, Salsa purity, the three parity/isolation invariants — code paths present; standing gates green.
- ❌ **Run-pipeline parity rule names a standing CI gate that does not exist.** Spec line 318 (and CLAUDE.md) say the gate is `cargo test -p smelt-runtime --test execute_parity`, "which runs the same fixture project through both `smelt-cli` and `smelt-ui` entry points and asserts identical model outputs, manifest contents, and selection sets." No `execute_parity.rs` target exists; only `compile_parity.rs` (compile API only, from outside the crate) and `select_parity.rs` (selection only, in-memory graphs). `compile_parity.rs:13-14` explicitly defers the end-to-end CLI↔UI test to "Phase 4 (`execute_parity.rs`)" — which never landed. The end-to-end CLI↔UI run-output parity invariant is therefore **unenforced**. → **BUG-001**.

### Invariant drift
- ✅ Invariants 1–8 (single CST, purity, no CST mutation, sync/async edges, lightweight dialect, no circular deps, total parse, unknown-key doctrine) — upheld by inspection + gates.
- ❌ Invariant 9 (canonical-address / one-path-one-entity) — the resolution-key half holds, but the *uniqueness* half (collision → hard error) is unenforced. See BUG-002.

### Timeless-oracle drift
- ✅ No plan-phase vocabulary in the spec body.
- ✅ `docs-site/docs/developing/architecture.md:211` "### Phase Ordering" is legitimate domain vocabulary (planner rule phases), not plan-phase leakage.
- Note: `compile_parity.rs`/`select_parity.rs` headers say "Phase 2"/"Phase 3"/"Phase 4" — these are test-file comments, outside the timeless-oracle scope (spec body + user docs), so not flagged.

### Freshness
- last_reviewed: 2026-05-05
- most recent change to referenced code: 2026-05-28 (`0a2bacc1` — canonical `smelt.<path>` addressing + `--scope`, #125)
- Verdict: mildly stale. The spec body already reflects canonical addressing (dated 2026-05-04), so content is current; recommend a light `/smelt:spec` touch to bump `last_reviewed` and reconcile the `execute_parity` gate name (BUG-001).

### Probe notes (to verify in later phases, not logged as bugs yet)
- A model referencing a `.csv` seed under a `paths:` root (`SELECT * FROM smelt.regions` with `models/regions.csv`) produced `"dependencies": []` in `explain --json` and no entity/diagnostic for the seed. Unclear whether this is `explain`'s output shape (seeds aren't "models") or a missing model→seed dependency edge. **Verify in C5 (seeds).**

### Summary
- Drift items: 2 (1 semantics — missing parity gate; 1 surface/invariant — collision unenforced).
- Both classified `needs-review`: BUG-001's resolution touches the run-pipeline parity invariant; BUG-002's resolution touches workspace-loading parity + the eager/lazy discovery split.
- Recommended next step: human review of BUG-001/BUG-002 (see ledger) — both are invariant-adjacent and not safe to auto-fix in-loop.
