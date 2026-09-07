# Phase 7d summary — Conformance cross-suite widening

## Shipped

- `SuccessionRecipe::contract: Option<ContractDecl>` + `with_contract(..)`
  combinator (`crates/smelt-maintenance-testkit/src/recipe/succession.rs`).
- `render.rs`'s `render_contract_block` generalized from `&ModelRecipe` to
  `Option<&ContractDecl>` so `render_succession_model_file` renders the same
  `contract:` block (`render.rs`, `render/succession.rs`).
- `gate_succession::assert_succession_equivalence_at_point` — the succession
  family's counterpart of `gate/partition_pool.rs`'s
  `assert_equivalence_at_point_with_frontier`, dispatching on
  `oracle_obligation`; `Exact`/`ExactOverRestrictedS` delegate to the existing
  unrestricted comparator, `ExactOverProcessedSWithLagBound` compares against
  the processed-source restriction plus `deferral::settled_lag_bound` over
  landed-but-unprocessed event times.
- `state_deletion.rs`: `succession_recipe_upholds_equivalence_with_state_deleted`
  — three windows (insert, late splice, delete) through a real
  `execute_project` run with `.smelt/` deleted between every step.
- `repair.rs`: `succession_full_refresh_repairs_a_perturbed_ledger_and_presented_table`
  — deliberately corrupts BOTH the presented table and the tombstone ledger,
  proves the oracle fails (non-vacuity), then proves `--full-refresh` restores
  both, the ledger checked against `emit_succession_ledger_rebuild_select`'s
  own output (never a hand-written comparandum).
- Three new unit tests: `succession_contract_block_renders_deferral`,
  `succession_deferral_recipe_is_admitted_not_refused`
  (`render/succession.rs`), and
  `succession_equivalence_at_default_point_matches_the_unrestricted_oracle`
  (`gate_succession.rs`).

## Decisions

- 2026-09-07: tests 6–7 (the contract-lattice `deferral` leg in
  `contract_points.rs`, driving a declared-deferral succession model to a
  licensed skip) are **not shipped**. Root cause below. Per the plan's own
  contingency clause, the test's expected assertion (a licensed skip) was not
  weakened to match actual (unlicensed) behaviour; the tests were left out of
  the tree entirely rather than committed red.
- Trimmed one line off `render_contract_block`'s doc comment to keep
  `render.rs` at its large-file baseline (1376) rather than bumping the
  ratchet for a one-line net growth.

## For the next planner

**Finding (blocks tests 6–7, not this phase's scope):** the succession
window-forward driver (`crates/smelt-runtime/src/maintenance_driver/succession/`)
never writes to `IntervalStore`. `contract_probes::resolve_deferral_frontiers`
reads a model's *maintained* frontier from `IntervalStore::get(model_name)`,
so for any succession model this is always `None`, and
`deferral::run_license(None, Some(_), d)` always falls through to `Run` —
`contract.deferral`'s run-skip license can never fire for a succession model
today, no matter how small the measured lag. This is criterion 3 work (the
plan's own contingency named this exact possibility): the plan-level
derivation already admits `contract.deferral` on a succession model (phase 3,
unaffected), but the *scheduler's* executed skip path silently never engages.
A future phase should either (a) have the succession driver record its own
maintained arrival frontier into `IntervalStore` after each successful fold,
or (b) teach `resolve_deferral_frontiers` a succession-aware frontier source.
Recommend re-adding this phase's tests 6–7 (preserved in this summary's plan
file, `phases/07d-plan.md`) once the fix lands — they were written and are
known-correct against the *intended* behaviour, just not executable yet.

Criterion 6's "the contract-lattice `deferral` leg includes one" clause is
therefore only partially met: the recipe-level `contract:` field and its
render/admission tests exist (shipped above), but the executed-skip
conformance leg is deferred.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-maintenance-testkit --quiet` — 65 passed
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 97 passed
  (full seeded sample green)
- `bash .claude/scripts/large-file-check.sh` — OK
