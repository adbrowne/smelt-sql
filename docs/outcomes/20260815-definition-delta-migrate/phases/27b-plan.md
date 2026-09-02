# Phase 27b plan — the region DELETE+INSERT family gains its conditional variant

## Objective

The region recompute family (`Technique::DeleteInsert`, the always-available whole-window
replace) rewrites its whole window unconditionally on every run, even where the cell has a
proven row identity and a fully comparable column group — the same P2/P3 proof that licenses
the column-scoped `MERGE`'s suppressed arm. Close that: resolve a region write variant purely
in `smelt-logical`, and emit the change-suppressed staged write from the one dispatch point
(`build_delete_insert_group_dispatched`) both the live executor and the dry-run/`explain`
reporter already share. Advances the "printed SQL cannot drift from executed SQL" and
conditional-maintenance success criteria.

**Established (do not re-derive):** the emission half is NOT the gap — `emit_staged_candidate_
conditional` and `emit_staged_candidate_conditional_recompute` both exist and the latter is
live-dispatched for `UpstreamMutation` membership cells (`execute_staged_membership_recompute`).
What is missing is (a) a region-family admission verdict and (b) its dispatch on the ordinary
`Trigger::NewData` region path.

**Design calls (recorded — do not re-litigate).**
1. **Realise the region variant with `emit_diff_patch`**, passing `slice_predicate =
   region.predicate(Some(table), partition_col)` and `DeleteLeg::Complete` (a region recompute's
   candidate covers its own slice by construction — the same grant `resolve_cell_choice` already
   makes for this corner). Rejected: `emit_staged_candidate_conditional` unmodified — its keyed
   `DELETE` is unbounded by the region and it has no departed-row leg, so a stored row whose key
   left the region would go stale and break equivalence; and a new fourth staged emitter, which
   would duplicate `emit_diff_patch`'s four legs for no semantic difference.
2. **Delta restriction wins over suppression** when both are admitted: the restricted arm narrows
   the scan itself (strictly cheaper), suppression only narrows writes within it. So the new arm
   sits after the `Restricted` arm in `build_delete_insert_group_dispatched`'s match.
3. **Fail closed to today's byte-identical widened scan** for: empty/`WholeRow` identity, an
   uncomparable group, a first-build/`ledger_catch_up` trigger (per `resolve_write_variant`'s
   existing posture), a model with no resolvable `DeltaRestrictionFacts`. A `technique: suppress`
   pin whose proof refused is a hard run error, never a silent fallback.

## Spec delta (make these edits first)

- `docs/specs/model_transforms.md` §"Change-suppressed MERGE and the staged-candidate conditional
  DELETE+INSERT" — one paragraph: the region family's conditional realisation is the slice-
  predicated staged write (update leg + complete delete leg + insert leg), equivalent to the
  unconditional region replace at fixed `S`, admitted by the same P2/P3 proof.
- `docs/specs/model_transforms.md` §Known Divergences — delete "The region DELETE+INSERT family
  still rewrites its whole window unconditionally."; leave the keyless-`EXCEPT ALL` and `write:`
  pin clauses (rows 27c/27d own them).
- `docs/specs/incremental_models.md` §Known Divergences "Conditional-maintenance gaps" — drop the
  `the region DELETE+INSERT family has no conditional variant` clause only.
- `docs-site/docs/reference/` — the page documenting maintenance statements/`--dry-run` output
  gains the matching user-facing sentence.

## Tests (red first)

- `crates/smelt-logical/src/maintenance/choice.rs` unit tests:
  - `region_write_variant_suppresses_over_a_proven_key_and_comparable_group`
  - `region_write_variant_is_unconditional_without_a_proven_key`
  - `region_write_variant_is_unconditional_on_a_first_build_trigger`
  - `region_write_variant_propagates_a_refused_suppress_pin` — `Err`, never `Unconditional`.
- `crates/smelt-runtime/tests/region_choice_ladder.rs::region_recompute_emits_the_conditional_
  staged_write_when_suppressible` — the dispatched group carries the `IS DISTINCT FROM` update
  leg and the region-predicated delete leg.
- `…::region_recompute_keeps_the_widened_scan_without_a_proven_key` — byte-identical to today's
  `emit_delete_insert` group (non-regression).
- `…::delta_restriction_wins_over_suppression_when_both_admit` — design call 2.
- `crates/smelt-runtime/tests/dry_run_statements.rs::dry_run_prints_the_region_conditional_form`
  — the dry-run report and the live dispatch agree (same call, no second resolution).
- `crates/smelt-runtime/tests/statement_parity.rs::region_conditional_write_matches_the_emitted_
  group_byte_for_byte` — executed-vs-emitted parity for the new family route.
- Live-equivalence leg (DuckDB, in `region_choice_ladder.rs` or the conformance harness): a
  second run over unchanged data leaves the region's contents equal to a full refresh, and a run
  where a key departs the region deletes that row (delete-leg coverage).

## Tasks

1. Make the spec + docs-site edits above.
2. Add `choice::RegionWrite { Unconditional { why }, Suppressed { key, compared_columns } }` and
   `resolve_region_write_variant(group_columns, comparability, row_identity, trigger,
   ledger_catch_up, overrides) -> Result<RegionWrite, ChoiceRefusal>` in
   `crates/smelt-logical/src/maintenance/choice.rs`, composed from the existing
   `resolve_write_suppression` + `resolve_write_variant` (no new proof logic).
3. Extend `DeltaRestrictionFacts` (`crates/smelt-runtime/src/maintenance_driver.rs`) with the
   resolved `RegionWrite`, derived from the same `Trigger::NewData` cell it already reads, so
   live and dry-run get it from one per-model derivation.
4. Add the third arm to `build_delete_insert_group_dispatched` (new `region_write` parameter),
   calling `emit_diff_patch` with the region slice predicate and `DeleteLeg::Complete`; update
   both call sites (`execute.rs`'s dry-run loop, `execute_delete_insert_with_delta_restriction`)
   and any test call sites.
5. Confirm the group's `transactional: true` reaches the backend through the existing
   `execute_statement_group` route (the staged temp relation must roll back as one unit).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance`
- `cargo test -p smelt-runtime --test region_choice_ladder --test dry_run_statements --test statement_parity --test delta_restricted_recompute --test technique_lowering`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance`
- `cargo test --workspace` (phase 25's summary: a shared-resolver change breaks tests outside the listed files)

## Commit message

`feat(maintenance): give the region DELETE+INSERT family its change-suppressed conditional variant`
