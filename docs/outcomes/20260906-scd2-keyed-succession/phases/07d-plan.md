# Phase 7d plan — Conformance cross-suite widening

## Objective

Close the last clause of success criterion 6 — "the `state_deletion.rs` and
`repair.rs` legs include succession recipes, and the contract-lattice `deferral`
leg includes one". Three new legs in the existing conformance suites, each
driving a `SuccessionRecipe` through the real `execute_project` pipeline; plus
the `contract:` field on `SuccessionRecipe` and its frontmatter rendering that
the deferral leg needs. Also advances criterion 3's `deferral`-is-admitted
clause from a derivation-level assertion to an executed one.

## Spec delta

None. No user-visible behaviour changes: `deferral` on a succession model is
already specified as admitted with unchanged frontier-lag semantics
(`incremental_shapes.md` §"Succession-grain design", pinned in phase 1), and the
`.smelt/`-deletion and `--full-refresh` rebuild claims are already specified.
This phase only makes them executable.

## Tests

Testkit (`crates/smelt-maintenance-testkit`):

1. `succession_contract_block_renders_deferral` (`render/succession.rs`) — a
   `SuccessionRecipe` with `contract: Some(ContractDecl::Deferral { days })`
   renders `contract:\n  deferral: 'N days'` into the model frontmatter, and a
   `contract: None` recipe renders byte-identically to today.
2. `succession_deferral_recipe_is_admitted_not_refused`
   (`render/succession.rs`, extending the existing `classify_succession_plan`
   helper) — a deferral-declared succession model still derives one
   `SuccessionPatch` cell with no refusal diagnostic (criterion 3's
   `deferral` clause, executed).
3. `succession_equivalence_at_default_point_matches_the_unrestricted_oracle`
   (`gate_succession.rs`) — harness self-check: the new
   `assert_succession_equivalence_at_point(.., &ContractPoint::Default)` is
   byte-for-byte the existing `assert_succession_equivalence_for` behaviour.

Conformance (`crates/smelt-cli/tests/maintenance_conformance`):

4. `succession_recipe_upholds_equivalence_with_state_deleted`
   (`state_deletion.rs`) — a delete-flagged succession recipe driven over three
   windows (including a late splice) with `StateDeletion::BetweenRuns`; oracle
   holds after every window and `nonempty_deletions_observed() > 0` (anti-vacuity,
   mirroring the existing legs).
5. `succession_full_refresh_repairs_a_perturbed_ledger_and_presented_table`
   (`repair.rs`) — drive two windows including a delete, then perturb BOTH the
   presented table (delete a row) and the tombstone ledger (delete a tombstone),
   assert the oracle now FAILS (non-vacuity), run `--full-refresh`, assert both
   relations are restored: presented matches the oracle and the ledger matches
   `emit_succession_ledger_rebuild_select`'s own result over the whole source.
6. `succession_deferral_recipe_upholds_restated_oracle_with_a_skipped_run`
   (`contract_points.rs`) — a `contract.deferral: 'N days'` succession model plus
   an undeclared succession sibling over the same source advances the landed
   frontier; the declared model's own run is then a licensed skip
   (`strategy == "skipped_deferral"`, `RunOutcomeKind::Skipped`), and
   `assert_succession_equivalence_at_point(.., &ContractPoint::Deferral { d })`
   holds throughout.
7. `succession_deferral_leg_is_not_vacuous` (`contract_points.rs`) — METAMORPHIC:
   the same post-skip state FAILS `ContractPoint::Default`, proving the relaxed
   oracle is genuinely relaxed rather than silently subsuming the strict one.

## Tasks

1. Add `pub contract: Option<ContractDecl>` to `SuccessionRecipe`
   (`recipe/succession.rs`), defaulting to `None` in both constructors, plus a
   `with_contract(..)` combinator.
2. Generalize `render.rs`'s `render_contract_block` to take
   `Option<&ContractDecl>` (keep the `ModelRecipe` call sites calling through) and
   render it from `render_succession_model_file` — test 1.
3. Widen `render/succession.rs`'s existing `classify_succession_plan` test helper
   coverage with test 2.
4. Add `assert_succession_equivalence_at_point(project, recipe, point,
   processed_arrival_frontier, input_frontier)` to `gate_succession.rs`:
   dispatch on `smelt_logical::contract::point::oracle_obligation` exactly as
   `gate/partition_pool.rs::assert_equivalence_at_point_with_frontier` does —
   `Exact`/`ExactOverRestrictedS` compare against the unrestricted oracle;
   `ExactOverProcessedSWithLagBound` compares against
   `render_succession_oracle_body_over` evaluated over the PROCESSED source
   restriction (`(SELECT * FROM main.sources_<n> WHERE <partition col> < DATE
   '<processed frontier>')` — reusing that function's existing relation seam, no
   second comparator), then calls
   `smelt_logical::contract::deferral::settled_lag_bound` over the
   landed-but-unprocessed event times read back from the source. Test 3.
5. Add the `state_deletion.rs` leg (test 4), reusing
   `stage_succession_recipe_for(...).with_state_deletion(StateDeletion::BetweenRuns)`
   and `drive_succession_window_and_assert_for` — no new drive loop.
6. Add the `repair.rs` leg (test 5), reusing `gate::snapshot_table_rows` and
   `tombstone_table_name`; the ledger's expected contents come from
   `smelt_logical`'s rebuild `SELECT`, never hand-written SQL.
7. Add the `contract_points.rs` legs (tests 6–7): stage the declared recipe, write
   the undeclared sibling with `render_succession_model_file`, drive with
   `request.select` to open the lag, assert the skip record and both oracles.
8. Run the gates; if any file crosses `.claude/large-file-baseline.txt` (or the
   1500-line cap), split it in the same commit rather than bumping the baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-maintenance-testkit --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` (full seeded
  sample green — the phase's headline gate)
- `bash .claude/scripts/large-file-check.sh`

Contingency: if the pre-loop deferral skip pass in
`crates/smelt-runtime/src/execute/project/mod.rs` turns out not to reach a
succession cell, that is criterion 3 work — do NOT weaken test 6. Record it in
the summary as a finding for the next planner to give its own phase row.

## Commit message

`test(succession): widen the state-deletion, repair, and deferral conformance legs`
