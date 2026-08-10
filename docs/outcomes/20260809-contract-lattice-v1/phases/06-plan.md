# Phase 6 plan — conformance oracle parameterised per lattice point

## Objective

Make `maintenance_conformance` assert each model against **its own** lattice point's oracle:
default cells against strict equivalence, `frozen_horizon` cells against the clamped-`S` oracle,
`deferral` cells against the bracketed oracle. The per-point restriction is a pure transform
single-owned in `smelt-logical::contract` that the gate *calls* (never re-derives), and two new
recipes exercise the relaxations so a relaxation is never silently tested as the default. Advances
success criteria 5 (and closes the gate half of criterion 3's single-owner claim).

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences → "The contract, plan, and graph layer"
(the bullet ending "Still missing: `maintenance_conformance` parameterisation per lattice point …
and `smelt explain` rendering …", ~line 2489): drop the gate-parameterisation half, leaving the
`explain` rendering gap and the per-cell `deferral` note. Gap-first phrasing, tracking link
unchanged. No Surface change — this phase adds no user-visible behaviour.

## Tests

Red-green, in order:

1. `crates/smelt-logical/src/contract/mod.rs` — `default_point_obligation_is_exact`: the default
   point's `OracleObligation` is `Exact` and leaves a run window untouched.
2. same file — `frozen_horizon_point_restricts_each_run_window`: `restrict_run_window` returns
   `clamp_write_range`'s narrowed start for `FrozenHorizon { h }`, never widening.
3. `crates/smelt-logical/src/contract/deferral.rs` — `settled_cutoff_drops_the_trailing_window`:
   cutoff = `input_frontier − D`; `None` for every non-deferral point.
4. `crates/smelt-logical/tests/contract_lattice_spec.rs` —
   `conformance_gate_consumes_the_oracle_transform`: structural — the `maintenance_conformance`
   sources call `smelt_logical::contract::{oracle_obligation, restrict_run_window,
   settled_cutoff}` and contain no local horizon/lag arithmetic (same shape as phase 5's
   `deferral_capabilities_are_single_owned`).
5. `crates/smelt-maintenance-testkit/src/s_tracker.rs` —
   `s_at_under_frozen_horizon_drops_the_late_frozen_row`: a row landing in an already-frozen
   partition is in `s_at` but absent from `s_at_for_point(FrozenHorizon)`.
6. `crates/smelt-cli/tests/maintenance_conformance/contract_points.rs` —
   `default_recipes_are_still_asserted_exactly`: harness self-check — a default recipe driven
   through `assert_equivalence_at_point(Default)` is byte-identically the pre-existing strict
   assertion (no silent weakening of the standing pool).
7. same file — `frozen_horizon_recipe_upholds_relaxed_oracle_and_not_the_default`: late row into a
   frozen partition — the relaxed oracle holds, and the *strict* comparison is asserted to FAIL
   (proof the relaxation is genuinely under test).
8. same file — `deferral_recipe_upholds_bracketed_oracle_with_a_skipped_run`: a schedule producing
   a licensed skip — the run manifest records `skipped_deferral`, and the bracket
   (`full_refresh(S_settled) ⊆ maintained ⊆ full_refresh(S)`) holds after every step.

## Tasks

1. `smelt-logical/src/contract/mod.rs`: add `ContractPoint { Default, FrozenHorizon { h },
   Deferral { d } }` (day-valued, matching the existing clamp/lag units) and
   `OracleObligation { Exact, ExactOverRestrictedS, Bracketed }`; `oracle_obligation(&point)`,
   `restrict_run_window(&point, start, end) -> (i64, i64)` (delegates to
   `frozen_horizon::clamp_write_range`), `settled_cutoff(&point, input_frontier) -> Option<i64>`
   (delegates to a new `deferral::settled_cutoff`). Pure, no I/O. Tests 1–3.
2. Extend `contract_lattice_spec.rs` with test 4.
3. Testkit `recipe.rs`: `ModelRecipe.contract: Option<ContractDecl>` (`frozen_horizon`/`deferral`
   interval strings), defaulting `None` so every existing recipe renders unchanged;
   `render.rs::render_model_file{,_with_edit}` emits the `contract:` block when present.
4. Testkit `s_tracker.rs`: `s_at_for_point(k, &ContractPoint)` applying `restrict_run_window`
   per recorded run before the existing per-window filter; `materialize_s_for_point` /
   `s_restricted_oracle_sql` reused unchanged; `s_at` becomes `s_at_for_point(_, Default)`.
   Test 5.
5. `maintenance_conformance/gate.rs`: `assert_equivalence_at_point(project, recipe, tracker, k,
   point)` dispatching on `oracle_obligation` — `Exact`/`ExactOverRestrictedS` are the existing
   both-direction `multiset_equal_via_backend` over the point-restricted `S`; `Bracketed`
   materialises two S sets (settled and full) and asserts one `EXCEPT ALL` direction against each.
   `assert_equivalence` delegates with `ContractPoint::Default` — the standing pool's behaviour is
   unchanged.
6. New `maintenance_conformance/contract_points.rs` (registered in `main.rs`) with the two relaxed
   fixtures and tests 6–8. Set `probes: cadence: off` on the frozen-horizon fixture and document
   inline *why*: the phase-3 late-arrival probe would fail the run on exactly the condition this
   test constructs (the probe has its own dedicated gate — phase 5's summary records the same
   fixture necessity for deferral).
7. Spec edit per §Spec delta.

## Verification

- `bash .claude/scripts/verify-phase.sh` (export `DUCKDB_LIB_DIR` + `LD_LIBRARY_PATH` inline).
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-maintenance-testkit`
- `cargo test -p smelt-cli --test maintenance_conformance` — the 67 pre-existing cases must stay
  green and unchanged in count-plus-new; report the new total.
- `cargo test -p smelt-runtime --test contract_deferral_probe --test contract_frozen_horizon_clamp
  --test contract_late_arrival_probe --test contract_deferral_schedule` — unregressed.

## Commit message

`feat(contract-lattice): conformance oracle parameterised per lattice point`
