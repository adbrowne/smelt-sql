# Phase 3c plan — gate hygiene: path drift after the large-file splits

## Objective

Three gates and one spec still cite single `.rs` file paths that this branch's
large-file splits turned into module *directories*, leaving the workspace suite red:
`contract_lattice_spec::frozen_horizon_triple_is_complete`,
`contract_lattice_spec::explain_contract_rendering_is_single_owned`, and
`state_docs_freshness::spec_references_are_live`. Fix each to resolve the *module*
rather than one file, so `verify-phase.sh` is unambiguously green for phases 4–10's
verification (the same reason phases 2a/2b/3b existed) and the drift class cannot
recur on the next split. This advances no success criterion directly; it unblocks
the ability to verify every criterion.

## Spec delta

No user-visible behaviour changes. One documentation citation is corrected:
`docs/specs/state.md` §References → **Code** — replace
`crates/smelt-logical/src/maintenance/availability.rs` with the live module directory
`crates/smelt-logical/src/maintenance/availability/`.

## Tests

Red-green, in this order:

1. `smelt-logical` `tests/support/module_source.rs` unit tests (new shared helper
   `read_module(repo_root, rel_stem) -> String`, resolving `<stem>.rs` if present,
   else concatenating every non-test `.rs` under `<stem>/`, reusing
   `test_only_files::is_test_only` to drop test-only files; panics loudly when
   neither form exists):
   - `read_module_resolves_a_single_file_module` — `<stem>.rs` form returns that file.
   - `read_module_resolves_a_split_directory_module` — `<stem>/mod.rs` + sibling
     production file are both present in the returned text.
   - `read_module_excludes_test_only_files` — a `#[cfg(test)] mod`-declared sibling's
     contents are absent from the returned text (so the gate cannot be satisfied by
     a symbol that only exists in tests).
   - `read_module_panics_when_the_module_is_absent` — `#[should_panic]`, naming the stem.
2. `contract_lattice_spec::frozen_horizon_triple_is_complete` — green again reading the
   `contract/frozen_horizon` module through `read_module` (all four `pub fn` legs).
3. `contract_lattice_spec::explain_contract_rendering_is_single_owned` — green again,
   with the ownership check strengthened: scan the whole `contract/` module for
   `pub fn effective_contract` and assert it is defined **exactly once**, rather than
   asserting it lives in `mod.rs`.
4. `contract_lattice_spec::gate_detects_a_missing_leg` — new negative proof that the
   rewritten reads still catch a real absence: `read_module` over a temp fixture
   module missing `pub fn clamp_write_range` does not contain the symbol (the gate's
   assertion would fire), so the fix did not turn the gate into a tautology.
5. `state_docs_freshness::spec_references_are_live` — green after the §References edit.

## Tasks

1. Add `crates/smelt-logical/tests/support/module_source.rs` with `read_module` +
   its four unit tests; include it via `#[path = "support/module_source.rs"]`
   alongside `test_only_files` in `contract_lattice_spec.rs` (it needs
   `test_only_files` too — include both, matching `walk_coverage/main.rs`'s pattern).
2. Rewrite `frozen_horizon_triple_is_complete` to read the module via `read_module`;
   leave the `contract/mod.rs` landing-status assertions as-is (that file still exists).
3. Rewrite `explain_contract_rendering_is_single_owned`'s first assertion to the
   exactly-once scan over `contract/`; leave the `smelt-cli/src/explain.rs` legs untouched.
4. Add `gate_detects_a_missing_leg`.
5. Grep the whole `contract_lattice_spec.rs` and `state_docs_freshness.rs` for any other
   `read("crates/.../x.rs")` whose path no longer exists; convert each to `read_module`.
6. Fix `docs/specs/state.md` §References → Code to cite `.../maintenance/availability/`.
7. Sweep `docs/specs/` for dead backtick-quoted `crates/…` citations
   (`grep -rhoE '\`crates/[A-Za-z0-9_./-]+\`' docs/specs/*.md` → test each for existence)
   and fix the mechanical `<x>.rs` → `<x>/` cases **only in this outcome's six anchor
   specs** (`incremental_shapes`, `incremental_models`, `model_properties`,
   `model_transforms`, `diagnostics`, `sources`, `state`), so phase 10's
   `/smelt:validate` pass is not fighting drift. Record the remainder in the summary.
8. Run the full workspace suite; record every still-red gate in the summary verbatim
   for phase 10 (do not fix unrelated failures here).

## Verification

- `cargo test -p smelt-logical --test contract_lattice_spec` — 14 tests green.
- `cargo test -p smelt-cli --test state_docs_freshness` — 4 tests green.
- `bash .claude/scripts/verify-phase.sh` — fmt + clippy (both feature sets) +
  full `cargo test` + `example_diagnostics`. This is the phase's real acceptance
  gate: the workspace suite must be green, or the summary must name each remaining
  red gate and why it is out of this phase's scope.

## Commit message

`fix(gates): resolve module directories instead of single files in three path-drifted gates`
