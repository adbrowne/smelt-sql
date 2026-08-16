# Phase 8 summary — conformance recipes for repair + diff-patch

## Shipped

- `RepairRecipe`/`RepairWriteMode`/`arb_repair_combiner` in
  `crates/smelt-maintenance-testkit/src/recipe.rs` — a typed recipe pairing a
  non-invertible `KeyedCombiner` (`Idempotent`/`OrderMonotone`), a fixed 3-day
  Form B band, and a write mode (`TargetedDeleteInsert` / `DiffPatch`).
- Rendering in `render.rs`: `render_repair_model_file`/`render_repair_source_yaml`/
  `render_repair_oracle_sql`/`stage_repair` — a clocked `mutation_profile:
  mutable_snapshot` source with a declared `unique_key`, generalizing
  `crates/smelt-runtime/tests/repair_lowering.rs`'s hand fixture into the
  generative pool.
- `RecordingBackend`/`RecordingBackendFactory` promoted from
  `smelt-runtime/tests/statement_parity.rs` into
  `smelt-maintenance-testkit/src/link_c_harness.rs` as `LinkCProject::
  run_recording` — needed because the repair family's live dispatch doesn't
  route through `RunReporter::maintenance_statements` yet, so
  `SqlCapturingReporter` alone can't see executed repair/diff-patch DML.
- `crates/smelt-cli/tests/maintenance_conformance/repair.rs` (new, 5 tests):
  admission/key-match, retraction-schedule equivalence, statement-execution
  proof (no silent full-refresh fallback), diff-patch reconcile equivalence
  + empty second-run diff, diff-patch statement labelling.
- Two `KnownBug` registry entries (`registry.rs`) with structural
  "still reproduces" checks, mirrored as Known Divergences entries in
  `docs/specs/incremental_models.md`.

## Decisions

- Reused `gate.rs`'s established idiom of small typed mutation helpers
  (`insert_repair_row`/`update_repair_row_amount`/`delete_repair_row`) plus
  fixed run-window helpers, rather than inventing a generic `RepairSchedule`
  DSL the plan's Tasks section sketched — matches how every other pool in
  this file drives its schedules.
- Promoted `RecordingBackend` to the shared testkit crate instead of copying
  `statement_parity.rs`'s private version, keeping one implementation.

## For the next planner

Two genuine production gaps surfaced by the gate, not test bugs — both
documented as `KnownBug` + spec Known Divergences, tracked to this outcome:

1. **Affected-key discovery under-approximates full-group deletion.**
   `repair_affected_keys_select` scans the *current* physical
   `mutable_snapshot` source within the run window; a key whose entire
   window contribution was deleted between runs leaves no trace to scan, so
   its stale output row is never repaired — violates obligation 7
   ("an under-approximation is never admissible"). The equivalence test's
   delete step had to target a key with a surviving row to avoid tripping
   this. **This is a correctness gap worth its own follow-up phase or
   outcome** — the repair family's core promise (retraction repairs
   correctly) is not fully upheld today.
2. **`repair_candidate_select` ignores decomposed-combiner hidden state.**
   A decomposed combiner (e.g. `OrderMonotone`'s `(v, o)` pair) has extra
   `__`-marked state columns in the physical table that the repair `INSERT`
   doesn't supply, so live repair errors for such combiners. Only
   `Idempotent` (plain `MAX`-shaped, no hidden state) gets the full
   equivalence-under-retraction loop; `OrderMonotone` only gets admission +
   creation-run coverage. **Also worth a follow-up** before the repair
   family is claimed complete for decomposed combiners.

Neither blocks phase 9 (surface/docs), since both are pre-existing runtime
gaps this phase's tests *discovered*, not regressions this phase introduced,
and both are now honestly documented rather than silently passing.

## Gates

- `bash .claude/scripts/verify-phase.sh` → ALL GREEN
- `cargo test -p smelt-cli --test maintenance_conformance` → 58 passed (5 new)
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering` → 21 + 10 passed, no regressions
