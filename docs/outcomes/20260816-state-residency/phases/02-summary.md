# Phase 2 summary — `state.mode` is consulted: posture-gated `.smelt/` writes

## Shipped

- `crates/smelt-logical/tests/contract_lattice_spec.rs`: repaired the pre-existing red gate
  recorded in the outcome's Blocked entry — the lattice-point-invariant lookup now targets
  `## Constraints & Invariants` instead of the removed `### The contract, plan, and graph
  layer` heading.
- `FileStore::new(project_dir, target, mode: StateMode)` — `smelt_core::config::StateMode` used
  directly (new `smelt-core` dep on `smelt-state`, `StateMode` made `Copy`), no duplicate enum.
  ~70 call sites updated across `smelt-state`/`smelt-cli`/`smelt-runtime`/`smelt-ui` and tests.
- `StateFamily` enum + private `FileStore::writes(&self, StateFamily) -> bool`
  (`crates/smelt-state/src/file_store.rs:26,195`) — single owner of the consequence table from
  `state.md` §"`state.mode` and what each posture provides", doc-cited by name. Every
  observability `save_*` is a no-op and every `load_*` returns the family default under an
  excluding posture. `save_reconciliation_store`/`load_reconciliation_store` stay fully
  ungated (correctness-class; doc comment cites phase 4).
- `stateless` skips `.smelt/` creation, the `meta.json` stamp, legacy-layout migration, and
  `.smelt/lock` entirely — `init()`/`lock()` are no-ops; `StateLock` is now an enum
  (`Held{file}`/`Noop`).
- `execute_project` builds `FileStore` from `config.state.mode`; same wiring threaded through
  every other `FileStore::new` site in `smelt-cli`/`smelt-ui`.
- `--resume` refuses by name under `stateless`, before the manifest scan
  (`crates/smelt-runtime/src/execute.rs:1155`), naming `state.mode` and the absent manifest.
- New tests: 6 unit tests in `smelt-state/src/file_store.rs`, 6 integration tests in new
  `crates/smelt-runtime/tests/state_posture.rs` (real `execute_project`, DuckDB backend).
- Fixed 6 `examples/web_analytics/tutorial_stages/*/smelt.yml` that implicitly relied on the
  old always-write behaviour for cross-stage `diff` schema detection — now declare
  `state.mode: intervals` explicitly.

## Decisions

- **2026-08-16.** Folded the `contract_lattice_spec` repair in as task 1 (per the outcome's
  own phase-2 decision log, option (b)) — done, verified `docs/specs/` untouched.
- **2026-08-16.** Discovered a *second*, distinct pre-existing red-gate class during the full
  `verify-phase.sh` run (see "For the next planner") and deliberately did **not** fold it in:
  unlike task 1's fix (a pure heading-pointer repoint), this one needs either a genuine
  `docs/specs/incremental_models.md` content addition (forbidden by this phase's "no
  `docs/specs/` edits" constraint) or a judgment call on test intent — out of this phase's
  scope, and the outcome's own precedent (adopting option (b) for task 1) doesn't extend to a
  fix requiring spec judgment, only a mechanical one.
- **2026-08-16.** `environments_run_writes_snapshot_store` test: no production code path calls
  `save_snapshot_store` from `execute_project` yet (fingerprint/environment reuse is
  out-of-scope machinery). The test runs a real `environments`-posture `execute_project` then
  directly exercises `FileStore::save_snapshot_store` against the same target/mode to prove the
  gating leg is unblocked — documented inline as a stand-in until that write path lands.

## For the next planner

- **New pre-existing red-gate class found (not this phase's target, not fixed):**
  `cargo test -p smelt-logical --test output_delta_spec
  graph_layer_states_typed_edges_and_narrowed_refusal` and `--test typed_edge_spec
  typed_edges_section_names_the_three_component_parts` both fail on current `main`/this branch
  independent of phase 2 (confirmed via `git stash`). Root cause: `incremental_models.md` now
  has **two** `### The graph layer` headings (an Overview mention at line 163, the real section
  at line 1258) post the `spec-redraft-incremental-models` merge (PR #166) — same first-match
  heading-lookup fragility as the outcome's original Blocked entry, but the fix isn't purely
  mechanical this time: even after pointing at the correct (second) section, one assertion in
  `output_delta_spec.rs` (`section.contains("General")`, checking the graph layer states the
  keyed-node refusal is scoped to a `General` verdict) still fails because the section's
  refusal prose uses lowercase `general` (the delta-signature verdict) rather than the
  capitalized `General` (the output-delta profile verdict) — these are two different lattice
  concepts that happen to share a spelling modulo case, and it's a judgment call whether the
  spec prose should name `General` explicitly there or whether the test's expectation is
  stale. Recommend a small follow-up phase/task: read `incremental_models.md` §"The graph
  layer" (line 1258) + `typed_edge_spec.rs` fully, decide with intent whether this is a spec
  gap or a stale test, then fix. Left both tests red and the code untouched to avoid scope
  creep into `docs/specs/` under this phase's constraint.
- Per-model `state.mode` narrowing (`smelt-fingerprint`'s `effective_mode`) is unchanged —
  confirmed out of scope by the plan.
- Phase 4 (engine-resident reconciliation ledger) is the next place `save_reconciliation_store`
  changes; this phase left it deliberately ungated.

## Gates

- `cargo fmt --all -- --check` — PASS
- `cargo clippy --all-targets` — PASS, zero warnings
- `cargo test -p smelt-logical --test contract_lattice_spec` — PASS (13 passed)
- `cargo test -p smelt-state` — PASS (263 + 2 + 6 passed)
- `cargo test -p smelt-runtime --test state_posture` — PASS (6 passed)
- `cargo test -p smelt-runtime --test execute_parity` — PASS (4 passed)
- `cargo test -p smelt-cli --test maintenance_conformance` — PASS (70 passed)
- `cargo test -p smelt-cli` (full) — PASS, including `example_diagnostics` (119 passed, 1 ignored)
- `cargo test --workspace --exclude smelt-cli --no-fail-fast` — PASS except the two
  pre-existing, unrelated failures documented above (confirmed pre-existing via `git stash`)
- `git diff --stat -- docs/specs/` — empty
- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/example_diagnostics PASS; the bundled
  `cargo test (workspace)` leg reports the same two pre-existing failures above (everything
  else green)
