# Phase 2 plan — Spec delta: Trino's maintenance surface stated

## Objective

State in `docs/specs/multi_backend.md` what phase 1 measured and what T3's residency verdict
implies: how a whole-row `MERGE` is spelled on Trino, how the absent `WHEN NOT MATCHED BY SOURCE`
clause changes the conditional-write lowering, and — per technique — which maintenance families
are reachable on a `trino` target and which take the degradation contract's downgrade or refuse.
Advances criteria 1 (the matrix confirmed and its consequences written down), 3 (the unreachable
families named, with the diagnostic each uses), and 11. Spec-only: no emitter or backend code
changes here.

## Spec delta

All three edits are in `docs/specs/multi_backend.md`.

1. **§"Whole-row MERGE"** — add Trino as a **third** spelling family, not a member of either
   existing one. DuckDB/Spark take `SET *` / `INSERT *`; GoogleSQL takes column-by-column `SET`
   plus `INSERT ROW`; Trino takes column-by-column on **both** arms — `SET *`, `INSERT *` and
   `INSERT ROW` are each a grammar-level parse error (quote the three measured messages from the
   decision log). So Trino, like BigQuery, reads `CompiledModel::output_columns`, and the existing
   empty-projection rule binds it identically: empty means *unknown*, and the emitter refuses with
   `UnsupportedOnBackend` rather than emitting a matched arm that assigns nothing.
2. **§"Column-scoped merge and conditional-write capabilities"** — under
   `supports_merge_not_matched_by_source`, record that Trino is now the second `false` backend and
   that the spec's existing consequence (departed-row delete emitted as a separate scoped `DELETE`
   inside the same statement group, never a refusal of the transform) applies there over a
   **non-atomic, `TargetSchema`-resident** group, so the three recovery obligations already stated
   for that group shape carry the departed-row delete too. Add the clause forms phase 1 measured
   **accepted**, since they are the vocabulary the merge-less conditional write is built from:
   `WHEN MATCHED THEN DELETE`, a `WHEN MATCHED AND <pred>` guard, two ordered `WHEN MATCHED` arms
   resolving first-match-wins, and a subquery `USING (SELECT … FROM <staged>)` source.
3. **§"Incremental & schema evolution per backend"** — extend the existing Trino paragraph with a
   per-technique table stating the landing state, derived from
   `smelt_logical::maintenance::availability` rather than asserted:

   | `Technique` | On `trino` | Diagnostic |
   |---|---|---|
   | `DeleteInsert` | reachable; the write window is emulated (`DELETE` + `INSERT`), since `supports_insert_overwrite` is `✗` | — |
   | `PerGroupRecompute`, no `key_scope` | reachable | — |
   | `PerGroupRecompute`, key-addressed (`UpstreamKeyed` / `DownstreamGrainOverUpstream`) | refused — needs the fingerprint sidecar, and a clamped current-source scan is unsound here, not merely wider | `UnsupportedOnBackend` |
   | `KeyedFold` | downgraded to its recompute-family equivalent — no reconciliation ledger, so no never-fold-twice refusal | `MaintenanceStateDowngraded` |
   | `ColumnScopedMerge`, `InPlaceUpdate` | downgraded — no transactional merge ledger, exactly as on Spark (Delta) | `MaintenanceStateDowngraded` |
   | `SuccessionPatch` | downgraded to `DeleteInsert` (full rebuild), never a ledger-less patch | `MaintenanceStateDowngraded` |

   State plainly that `supports_column_scoped_merge = ✓` and this row are **not** in conflict: the
   flag describes a statement shape Trino can execute, while the plan cell's technique demands a
   correctness structure the dialect does not realise, and the second question is asked after the
   first. Close with: Trino introduces **no new diagnostic code** — the three existing codes
   (`MaintenanceStateDowngraded`, `UnsupportedOnBackend`, `DeclaredContractRequiresState`) cover
   every route above, so no later phase may mint a Trino-specific one.

## Tests

New file `crates/smelt-cli/tests/trino_incremental_spec_freshness.rs`, following
`trino_emission_spec_freshness.rs`'s `read_spec`/`section` shape:

- `whole_row_merge_states_trinos_named_column_form` — §"Whole-row MERGE" names Trino and rejects
  all three star/`ROW` shorthands, and names `output_columns` + the empty-projection refusal.
- `conditional_write_section_states_trinos_absent_not_matched_by_source` — the separate scoped
  `DELETE` consequence and the non-atomic `TargetSchema` group are both stated for Trino.
- `conditional_write_section_lists_the_measured_accepted_clause_forms` — the four accepted forms
  appear, so a later phase cannot quietly build on an unmeasured one.
- `incremental_section_states_a_landing_state_for_every_technique` — every `Technique` variant
  spelling appears in the §"Incremental & schema evolution per backend" table.
- `spec_downgrade_table_matches_the_pure_availability_functions` — the prose is checked against
  code, not just itself: `realisable_state_structures(SqlDialect::Trino)` is empty, and for each
  technique the spec's verdict column agrees with `required_state_structure`'s mapping (a
  structure-requiring technique reads "downgraded"/"refused", `DeleteInsert` reads "reachable").
- `no_trino_specific_diagnostic_code_is_introduced` — the section names the three existing codes
  and no `*Trino*` code, and each named code exists in `docs/specs/diagnostics.md`'s catalogue.

## Tasks

1. Write the six tests first; confirm they fail against today's spec text.
2. Edit §"Whole-row MERGE" (delta 1).
3. Edit §"Column-scoped merge and conditional-write capabilities" (delta 2).
4. Edit §"Incremental & schema evolution per backend" (delta 3), deriving each row from
   `availability/state_structure.rs` rather than from the outcome's prose.
5. Confirm no §Surface matrix cell changes (phase 1 measured none) and leave the table alone.
6. Re-run the new gate green; re-run the two existing Trino spec-freshness gates unchanged.

## Verification

- `cargo test -p smelt-cli --test trino_incremental_spec_freshness`
- `cargo test -p smelt-cli --test trino_spec_freshness --test trino_emission_spec_freshness`
- `cargo test -p smelt-core --test trino_docs_freshness`
- `bash .claude/scripts/large-file-check.sh`
- `bash .claude/scripts/verify-phase.sh`

No live tier is needed for this phase; nothing here may skip.

## Commit message

`docs(trino): state Trino's maintenance surface — MERGE spelling, conditional-write lowering, per-technique landing state`
