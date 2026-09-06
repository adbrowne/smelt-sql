# Phase 3b summary — the test-file blind spot in three structural gates

**Shipped:**
- `crates/smelt-logical/tests/support/test_only_files.rs`: pure `is_test_only(repo_root, rel_path)` / `declared_cfg_test(parent_src, stem)` — a file is test-only when its parent module source (`<dir>/mod.rs`, else the sibling `<dir-name>.rs`) declares `mod <stem>;` under `#[cfg(test)]` (same line or nearest non-blank line above), applied transitively up the directory chain. Fails loud: no discoverable parent declaration ⇒ production.
- `join_context_reach.rs` and `walk_coverage.rs` now include the shared module via `#[path = "support/test_only_files.rs"]` and filter `scanned_files` through it. Added `gate_scans_production_walk_sources` / `gate_scans_production_choice_sources` scanned-set regression tests.
- `.claude/scripts/hardening-budget.sh`: `_is_test_only_file()` (bash/awk twin of the same rule) called from `_count_crate` alongside the existing `tests.rs`/`tests/` skips; header comment updated.
- `crates/smelt-core/tests/hardening_budget.rs`: new `cfg_test_declared_module_files_are_not_counted` (Test C) — fake `src/m/mod.rs` declaring `#[cfg(test)] mod helper_tests;` + `mod real;`; asserts only `real.rs`'s `.unwrap()` counts.

**Decisions:**
- No baseline update was needed. `.claude/hardening-baseline.txt`'s `smelt-logical expect 1` already reflected the *correct* (post-fix) count — the fix restores the gate's own counting to match an already-correct baseline, rather than the baseline needing to tighten. Ran `--update` once to check, confirmed zero diff against the committed file, and reverted to the original (preserving its dated sign-off comment history) rather than keep a no-op rewrite.
- Classification is derived from the `mod <stem>;` declaration site, not a `*_tests.rs` naming convention, per the plan — proven by the `nested_under_test_only_module_is_test_only` transitivity test and Test C using a non-conventionally-named `helper_tests.rs`.

**For the next planner:**
- Phase 3c's target failure (`state_docs_freshness::spec_references_are_live`, citing the vanished `crates/smelt-logical/src/maintenance/availability.rs`) is still red after this phase, as the phase 3b plan anticipated — untouched by this phase's work, left for 3c.
- Did not touch `contract_lattice_spec::frozen_horizon_triple_is_complete` / `::explain_contract_rendering_is_single_owned` (also phase 3c's path-drift class) — not re-verified here; phase 3c should confirm their current state independently since this summary only ran the full workspace suite once and observed the one failure above.

**Gates:**
- `cargo test -p smelt-logical --test join_context_reach --test walk_coverage` — pass (21 tests).
- `cargo test -p smelt-core --test hardening_budget` — pass (5 tests).
- `bash .claude/scripts/hardening-budget.sh` — pass, no baseline diff.
- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/example_diagnostics pass; workspace `cargo test` fails only on `state_docs_freshness::spec_references_are_live` (phase 3c's target, pre-existing, unrelated to this phase's changes).
