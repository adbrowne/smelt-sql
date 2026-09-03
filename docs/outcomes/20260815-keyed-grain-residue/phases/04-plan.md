# Phase 4 plan — Derive and print the execution postures

## Objective

Advance success criterion 4: turn the three model-level properties of
`incremental_shapes.md` §"Derived execution postures" into one pure derivation in `smelt-logical`
(the single owner), have the runtime's existing re-run-tolerance verdict (`ledger_grade`) consume
it rather than re-derive it, and print all three — plus the derived run shape they qualify —
in `smelt explain`'s maintenance-plan report (text and `--json`).

## Spec delta (applied first)

`docs/specs/incremental_shapes.md`:

1. §"Derived execution postures" — **Order-independence**: make the qualifying enumeration
   explicit rather than a partial gloss. The formal rule ("holds iff every combiner is
   order-independent") already decides the case: additive fold (`+`, `XOR`) is commutative and
   associative and therefore qualifies — the section already admits decomposed fold, whose own
   state columns are additive. Restate as: extremal/lattice, additive fold, decomposed fold and
   proven once-write qualify; order-monotone overwrite and plain overwrite do not. This is a
   clarification of the stated rule, not a widening of admission — re-run tolerance (which
   additive fold still fails) is the separate posture.
2. §Known Divergences — rewrite the "The derived execution postures are internal, and one of the
   three is not derived at all" bullet: the verdict is now derived and printed; what remains is
   that the windowed-keyed driver still applies windows sequentially even where order-independence
   holds (an unused optimisation, not a correctness gap).

`docs-site/docs/reference/smelt-explain.md` — document the new `Execution postures:` block
alongside the existing `State columns:` / `Key temporal locality:` blocks (run shape line + the
three verdicts, each with its reason).

## Tests (red first)

New `crates/smelt-logical/tests/execution_postures.rs`:
- `additive_sum_model_is_not_rerun_tolerant_but_is_order_independent` — a `SUM` column: re-run
  tolerance false (naming the additive column), order-independence true.
- `order_monotone_column_forces_sequential_application` — `MAX_BY`: re-run tolerant, but
  order-independence false and the reason names the offending column (§"Ordering ties").
- `lattice_once_write_and_decomposed_columns_are_order_independent` — `MAX` + `COALESCE` +
  a state-bearing (`AVG`) column: both re-run tolerant and order-independent.
- `plain_overwrite_is_order_dependent` — `ANY_VALUE` (snapshot-reconcile family): order-independence
  false.
- `reprocessing_refusal_holds_for_every_family` — the refusal verdict is unconditional across all
  the above classifications.

`crates/smelt-runtime/src/cumulative.rs` unit tests:
- `ledger_grade_agrees_with_shared_posture_derivation` — over a mixed additive+idempotent column
  set, `ledger_grade()` is exactly the shared derivation's re-run-tolerance verdict (delegation,
  not a parallel rule).

`crates/smelt-cli/tests/explain_maintenance.rs`:
- `explain_prints_execution_postures_for_keyed_model` — text output contains an
  `Execution postures:` block with the run shape and all three verdicts.
- `explain_json_carries_execution_postures` — the `--json` payload carries the same three verdicts.
- `explain_omits_execution_postures_for_non_keyed_model` — a `grain: partition` model prints no
  postures block (nothing to classify).

## Tasks

1. Apply the two `incremental_shapes.md` spec edits above.
2. In `crates/smelt-logical/src/rules/cumulative.rs`, next to `state_column_summary`, add
   `pub struct ExecutionPostures { rerun_tolerant, order_independent, reprocessing_refused, ... }`
   — each verdict a bool plus a short `reason: String` naming the deciding column/family — and
   `pub fn execution_postures(columns: &[AggregatorColumn]) -> ExecutionPostures`, taking the
   column slice (not the whole classification) so the runtime rule can call it too. Derive:
   re-run tolerance = no `Sum`/`BitXor` combiner in any column or its state columns (the rule
   `ledger_grade` states today); order-independence = no `OrderMonotone`/`PlainOverwrite` column;
   reprocessing refusal = always, per §"Reprocessing". Add a convenience
   `CumulativeClassification::execution_postures()` and re-export from `smelt-logical/src/lib.rs`.
3. Rewrite `WindowedKeyedRule::ledger_grade` in `crates/smelt-runtime/src/cumulative.rs` to
   delegate: `if execution_postures(&self.aggregator_columns).rerun_tolerant { Idempotent } else
   { Additive }`, keeping the existing doc comment's rationale but pointing at the single owner.
4. Add `pub execution_postures: Option<ExecutionPostures>` to `MaintenancePlanResult`
   (`crates/smelt-db/src/queries/maintenance.rs`), defaulted `None` at every construction site,
   and populate it in `smelt-db/src/lib.rs`'s `maintenance_plan_report` from the classification it
   already builds for `state_columns` (same call, no second classify).
5. Print the block in `crates/smelt-cli/src/explain.rs` immediately before `State columns:`:
   `Execution postures:` with `run shape:` (window-forward / snapshot-reconcile, from
   `is_snapshot_reconcile`), then `re-run tolerance:`, `order-independence:`, `reprocessing:` —
   each `yes`/`no` plus its reason. Omit the whole block when `execution_postures` is `None`.
6. Thread the same value through `build_maintenance_plan_json` (`explain.rs`) and its caller
   (`crates/smelt-cli/src/commands/explain.rs`), mirroring how `state_columns` is passed.
7. Update `docs-site/docs/reference/smelt-explain.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test execution_postures --test keyed_families`
- `cargo test -p smelt-runtime --lib cumulative`
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test cli_docs_coverage`
- `cargo test -p smelt-cli --test maintenance_conformance` (grading unchanged end-to-end)

## Commit message

`feat(incremental): derive the keyed execution postures once and print them in smelt explain`
