# Phase 5 plan — The two invariants as standing tests

## Objective

Turn the outcome's two governing invariants into gates that fail when broken, rather than
spot-checks that happen to pass. **Claim ⇒ builder** currently compares two hand-maintained
tables (`realisable_state_structures` vs. `has_emitters` in
`maintenance_availability/realisation.rs`); phase 5 re-derives the right-hand side by *calling
the real builders*, and enumerates them so a new one cannot appear unclassified. **Absence ⇒
downgrade** is currently three hand-picked cells; phase 5 makes it exhaustive over `Technique`
× key-scope shape, and confines the refusal sites to an allowlist. Advances success criteria 3
and 4.

## Spec delta

None. Phase 2 landed the normative text (`state.md` §"Which dialects realise which structure",
§"The degradation contract") and phase 4 corrected `multi_backend.md` §"Parity contract". This
phase adds gates for behaviour already specified; no user-visible surface changes.

## Tests

New module `crates/smelt-runtime/tests/availability_seam/builders.rs` (registered in
`availability_seam/main.rs` — that binary already depends on both `smelt-state` and
`smelt-logical`, so no `Cargo.toml` change):

1. `every_state_builder_entry_point_is_classified` — structural. Scan
   `crates/smelt-state/src/{ledger,observed_delta,tombstone}.rs` for `pub fn` items whose return
   type is the dialect-refusing result (`LedgerResult`, the observed-delta and tombstone
   equivalents) and assert each name appears in this file's census array mapping entry point →
   `StateStructure`. A new or renamed builder that nothing classifies fails, naming it;
   a census row naming a function no longer present fails from the other side.
2. `a_builder_answers_exactly_when_its_structure_is_claimed` — two-sided, over `ALL_DIALECTS` ×
   census. Call the real builder with sample arguments and assert `is_ok()` equals
   `realisable_state_structures(dialect).contains(&structure)`. This replaces the restated
   `has_emitters` table as the source of the right-hand side for the four builder-backed
   structures (`has_emitters` stays only for the `FingerprintSidecar` row, which has no
   `smelt-state` builder — its realisation gate is the capability flag, covered by test 3).
3. `the_sidecar_claim_matches_every_backend_capability` — widen
   `realisation.rs::the_sidecar_claim_matches_the_backend_capability`'s loop to include
   `BackendCapabilities::trino_iceberg()`, which it currently omits, so Trino's sidecar absence
   is checked against the flag `maintenance_driver/sidecar.rs` actually gates on.
4. `state_builder_call_sites_stay_inside_the_gated_modules` — structural. Every production call
   site of a census entry point lives in `smelt-runtime/src/maintenance_driver/`,
   `smelt-runtime/src/execute/project/ledger_reset.rs`, `smelt-runtime/src/execute/key_addressed.rs`,
   `smelt-backend-bigquery/src/`, or `smelt-state/src/` itself. A call from anywhere else is a
   path the `required_state_structure` gating argument does not cover, and fails naming the file.

New module `crates/smelt-logical/tests/maintenance_availability/trino_invariants.rs`:

5. `every_technique_downgrades_or_needs_nothing_on_trino` — exhaustive over `Technique` (kept
   exhaustive by a `match` that a new variant breaks) × the three `KeyDiscovery` shapes plus the
   no-key-scope shape. Under Trino availability: a cell whose pre-resolution
   `required_state_structure` is `None` is unchanged with no downgrade; every other cell resolves
   to `recompute_equivalent`, carries a `StateDowngrade` whose `original` is the ideal technique
   and whose `missing` is the pre-resolution required structure, and post-resolution
   `required_state_structure` is `None`. The last clause is the reachability half: no builder is
   reached because no resolved cell requires one.
6. `the_ideal_plan_survives_resolution_on_trino` — non-vacuity + the resolve-late requirement:
   the same fixtures under `StateAvailability::all()` keep their ideal techniques, so test 5 is
   not passing because the fixtures were never structure-bearing.

Extend `crates/smelt-cli/tests/trino_explain_downgrade.rs`:

7. `explain_on_trino_downgrades_every_structure_bearing_shape` — stage a Trino-target project
   with one model per structure-bearing shape (keyed fold, column-scoped merge, key-addressed
   per-group recompute, succession patch), run `smelt explain --json`, assert exit 0, assert each
   model's JSON carries a `state_downgrade` naming its missing structure, and assert the output
   contains none of the `Unsupported*Dialect` refusal messages.

## Tasks

1. Add `builders.rs` to `availability_seam/main.rs`'s module list; write the census array
   (entry point name, sample-argument call, `StateStructure`).
2. Red-green tests 1, 2, 4; fix anything they surface in `smelt-state`'s dispatch or a call site.
3. Trim `realisation.rs::has_emitters` to the `FingerprintSidecar` row only, updating its doc
   comment to say the other four rows are now builder-derived by `builders.rs`, and widen the
   capability loop (test 3).
4. Add `trino_invariants.rs` to `maintenance_availability/main.rs`; write the exhaustive
   generator and tests 5 and 6.
5. Extend the staged fixture in `trino_explain_downgrade.rs` to the four shapes; write test 7.
6. Run the gates; if a baseline needs a bump, record the sign-off note in the summary rather than
   trimming a test.
7. Write `phases/05-summary.md` (shipped / decisions / for the next planner / gates).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test availability_seam`
- `cargo test -p smelt-logical --test maintenance_availability --test state_realisability_docs`
- `cargo test -p smelt-cli --test trino_explain_downgrade --test trino_spec_freshness`
- `cargo test -p smelt-state --test ledger_dialect`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`test(state): the two Trino invariants as standing gates — claim ⇒ builder derived from the builders, absence ⇒ downgrade exhaustive over Technique`
