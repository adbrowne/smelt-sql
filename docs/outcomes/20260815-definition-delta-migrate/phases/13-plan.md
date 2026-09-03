# Phase 13 — Write-pin equivalence: real column-comparability + a pre-execution write-variant gate

## Objective

Close success criterion 12's two halves. (a) The per-cell equivalence hook passed to
`resolve_write_pin` stops being `|_pattern| Ok(())`: a compare-based write pattern
(`diff_patch`, `keyed_conditional`, `staged_candidate`) is refused pre-run when the cell's own
derived column comparability (P3) cannot uphold it. (b) An inadmissible write-*variant* pin
(`technique: suppress` over a cell whose suppression proof refused) refuses the run instead of
silently falling back to region recompute, and `smelt explain` shows that refusal for the P3 case,
not only the decidable `WholeRow` case.

## Spec delta (made first, by the implement step)

`docs/specs/incremental_models.md`:
- §"Per-cell write addressing" → "User pins": state that a `write:` pin's equivalence factor is
  evaluated against the cell's **derived** column comparability and row identity (the same P2/P3
  proof `resolve_write_suppression` owns), not structural facts alone; a compare-based pattern over
  an incomparable group refuses with `MaintenanceWriteAddressingRefused`.
- Same section (variant dimension): a hard `technique: suppress` pin whose suppression proof
  refused is a run-refusing `ChoiceRefusal` before any statement executes, and `smelt explain`
  prints it — never a silent fallback to the unconditional/region form.
- §Known Divergences: delete "The write-pin equivalence factor is structural only" and
  "An inadmissible write-*variant* pin has no pre-execution gate"; if any residue survives
  (e.g. a pattern family with no comparability-based obligation), narrow rather than delete.

## Tests (red-green)

`crates/smelt-logical` (new `maintenance/mod.rs` or `choice.rs` unit tests):
1. `compare_based_pattern_refuses_an_incomparable_group` — `cell_equivalence_proof("diff_patch", …)`
   returns `Err` naming the incomparable column.
2. `compare_based_pattern_accepts_a_fully_comparable_group` — same call returns `Ok`.
3. `region_and_full_rebuild_patterns_need_no_comparability_proof` — `Ok` with empty comparability.
4. `compare_based_pattern_refuses_a_whole_row_cell` — P2 failure propagates through the same hook.

`crates/smelt-db/tests/maintenance_write_pin_diagnostics.rs`:
5. `diff_patch_pin_over_an_incomparable_group_reports_addressing_refused` — RED today (hook always
   accepts); asserts `MaintenanceWriteAddressingRefused` naming the cell and the column.
6. `write_pin_over_a_comparable_group_still_reports_nothing` — no false refusal on the existing
   admissible fixture.

`crates/smelt-cli/tests/maintenance_pins.rs`:
7. `suppress_pin_over_a_refusing_cell_fails_the_run` — a `technique: suppress` cell whose P2/P3
   proof refuses makes `execute_project` error with the `ChoiceRefusal`/
   `MaintenanceUnboundedFootprint` text instead of completing via region recompute.
8. `explain_reports_a_refused_suppress_pin_for_a_p3_failure` — `smelt explain` surfaces the same
   refusal for an identity-bearing cell with an incomparable compared column.

## Tasks

1. Land the spec delta above.
2. Add `pub fn cell_equivalence_proof(pattern: &WritePattern, group_columns: &[String],
   comparability: &[ColumnComparability], row_identity: &RowIdentityVerdict) -> Result<(), String>`
   to `crates/smelt-logical/src/maintenance/mod.rs` — single owner; compare-based patterns delegate
   to `choice::resolve_write_suppression` and map `Unconditional { why }` to `Err(why)`; every other
   registry pattern is `Ok`. Tests 1–4.
3. Thread the derived comparability out of the plan derivation: add
   `pub comparability: Vec<ColumnComparability>` to `MaintenancePlanResult`
   (`crates/smelt-db/src/queries/maintenance.rs`), populated from the single
   `analysis::walk::model_property_vector` call the derivation already makes — consumers read it,
   never re-walk (maintenance-plan purity).
4. Replace `write_pin_diagnostics`' `|_pattern| Ok(())` with `cell_equivalence_proof` bound to the
   matched `plan_cell`'s group columns, `comparability`, and `row_identity`; extend the function's
   signature with the comparability slice and update its caller. Tests 5–6.
5. In `crates/smelt-runtime/src/maintenance_driver.rs`, convert both write-variant
   `let Ok(..) = resolve_write_variant(..) else { continue }` sites (~lines 987 and 1286) so a
   `ChoiceRefusal` propagates as a run error (`anyhow!(refusal.to_string())`) while every non-refusal
   path keeps its current behaviour; delete the "REAL silent fallback" comment block that documents
   the gap. Test 7.
6. In `crates/smelt-cli/src/explain.rs`, use the new `result.comparability` + the cell's group
   columns to call the real `resolve_write_suppression`/`resolve_write_variant` for the
   identity-bearing case too, replacing the `facts.has_identity` proxy branch's best-effort prints
   with the authoritative verdict and propagating a refusal as an `explain` error; update the two
   "no `sql`/`JoinContext` threaded" comments. Test 8.
7. Re-check `crates/smelt-cli/tests/maintenance_conformance/gate.rs:2615`'s note about
   `resolve_write_variant` still describing reality; adjust the comment if not.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-db --test maintenance_write_pin_diagnostics`
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_pins --test maintenance_conformance --features duckdb`

## Commit message

`feat(maintenance): prove write pins against derived column comparability and refuse inadmissible write-variant pins before execution`
