# Phase 2 plan — `state.mode` is consulted: posture-gated `.smelt/` writes

## Objective

Make `state.mode` a runtime input rather than a parsed-and-ignored key. `FileStore` — the single
choke point for every `.smelt/` write — carries the project posture, and each observability
family is written only under a posture that includes it, so a `stateless` project never creates
`.smelt/` at all. Advances success criterion 1. Also repairs the pre-existing red standing gate
recorded in the outcome's Blocked entry, so nothing is stacked behind a broken gate.

## Spec delta

None. The behaviour is already normative in `docs/specs/state.md` §"`state.mode` and what each
posture provides" (the consequence table), §"The optionality rule", and `docs/specs/run_state.md`
§Semantics "Stateless writes nothing". This phase makes the implementation match; it must not
edit those sections. The `.smelt/`-resident reconciliation ledger stays a live Known Divergence
(closed by phase 4), so `state.md` §Known Divergences is untouched here too.

## Tests

Repair (task 1):

- `smelt-logical/tests/contract_lattice_spec.rs::constraint_and_claude_md_state_the_lattice_invariant`
  — re-point the lookup at `## Constraints & Invariants` (the lattice-point invariant is a bullet
  post-redraft, not a `###` subsection); assert the same `lattice point` + `smelt-logical`
  substrings. Red before, green after, with zero `docs/specs/` edits.

New `crates/smelt-state/src/file_store.rs` unit tests:

- `stateless_store_creates_no_directories` — constructing a `stateless` `FileStore` and calling
  every `save_*` for an observability family leaves the project dir with no `.smelt/`.
- `stateless_lock_is_a_noop` — `acquire_lock` under `stateless` succeeds without creating
  `.smelt/lock`.
- `intervals_posture_writes_its_families_but_not_snapshots` — manifests, reports, intervals,
  landed deltas, schemas, source postures, frozen-band baselines all land; `save_snapshot_store`
  is a no-op.
- `environments_posture_writes_every_family` — including the snapshot store.
- `excluded_family_loads_as_empty_not_error` — `load_*` for a posture-excluded family returns the
  default store rather than an error, so consumers degrade instead of failing.
- `reconciliation_store_ignores_the_posture` — `save_reconciliation_store` writes under
  `stateless`; it is correctness-class (`state.md` §"The state-structure inventory") and stays
  ungated until phase 4 moves it engine-resident. Guards against over-gating.

New `crates/smelt-runtime/tests/state_posture.rs` (real `execute_project`, DuckDB backend):

- `stateless_run_creates_no_smelt_dir` — a project with `state.mode: stateless` and no
  reconciliation-graded model runs green and leaves `.smelt/` absent.
- `stateless_run_writes_no_manifest_or_report` — same run, asserted at the family level.
- `intervals_run_writes_manifest_intervals_and_schemas` — the `intervals` families appear.
- `intervals_run_writes_no_snapshot_store` — `snapshots.json` absent under `intervals`.
- `environments_run_writes_snapshot_store` — present under `environments`.
- `resume_under_stateless_refuses_naming_the_posture` — `--resume` with `state.mode: stateless`
  fails with a message naming `state.mode` and the absent manifest (refuse-loudly-by-name arm of
  §"The optionality rule"), not the generic "no partially-failed run" text.

## Tasks

1. Fix `contract_lattice_spec.rs`'s heading lookup; confirm `cargo test -p smelt-logical --test
   contract_lattice_spec` is green and `git diff -- docs/specs/` is empty.
2. Add `smelt-core = { path = "../smelt-core" }` to `crates/smelt-state/Cargo.toml` and use
   `smelt_core::config::StateMode` directly — no duplicate enum, no conversion to drift. (`cargo`
   proves acyclicity: `smelt-core` depends only on `smelt-parser`/`smelt-types`.)
3. Give `FileStore` a `mode: StateMode` field and change `FileStore::new(project_dir, target,
   mode)`; update all ~64 call sites, passing `StateMode::Environments` in tests that assert
   pre-existing write behaviour.
4. Add one private `fn writes(&self, family: StateFamily) -> bool` on `FileStore` — the single
   owner of the gating, a direct transcription of `state.md`'s consequence table, with a doc
   comment citing that section by name. No `save_*` re-derives the rule.
5. Make every observability `save_*` a no-op under an excluding posture and every `load_*` return
   the family's default; leave `save_reconciliation_store`/`load_reconciliation_store` ungated
   with a doc comment citing the residency divergence and phase 4.
6. Skip `.smelt/` creation, the `meta.json` version stamp, the legacy-layout migration, and
   `.smelt/lock` acquisition entirely under `stateless`.
7. In `execute_project`, build the `FileStore` from `config.state.mode` (already in scope as
   `config: Arc<Config>`) and pass the same posture at every other construction site in
   `smelt-cli`/`smelt-ui`. Per-model `state.mode` narrowing keeps governing only snapshot reuse
   (`smelt-fingerprint`'s `effective_mode`) — unchanged in this phase.
8. Make `--resume` refuse by name under `stateless` before the manifest scan runs.
9. Re-check `examples/` run clean (the examples' declared postures decide which `.smelt/`
   artifacts they now produce).

## Verification

- `bash .claude/scripts/verify-phase.sh` (full — must be green, including the gate repaired in
  task 1)
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-state`
- `cargo test -p smelt-runtime --test state_posture`
- `cargo test -p smelt-runtime --test execute_parity` (run-pipeline parity invariant)
- `cargo test -p smelt-cli --test maintenance_conformance` (equivalence is posture-independent)
- `git diff --stat -- docs/specs/` — empty

## Commit message

`feat(state): consult state.mode at runtime so postures gate .smelt/ writes`
