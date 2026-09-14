# Phase 2 plan — state the posture in the specs, and gate it against the code

## Objective

Turn phase 1's measurement into normative text: `state.md`'s realisability table gains a Trino
column reading **no** five times with the reason stated as permanent, and
`multi_backend.md` §"Incremental & schema evolution per backend" states the same for the `trino`
target. A new standing gate ties the table to `realisable_state_structures` so the two cannot
drift. Advances success criteria 2 (the statement) and, by naming the two diagnostics the
degradation path uses, prepares 4 and 5. Spec-only plus its gate — no runtime wiring, no live tier.

## Spec delta

Spec-first; the implement step makes these edits.

1. **`docs/specs/state.md` §"Which dialects realise which structure"** (line ~111) — the table
   gains a `Trino (Iceberg)` column reading **no** on all five rows. The paragraph below it, which
   today explains Spark's permanent absence, is extended with Trino's: the Iceberg connector
   accepts writes **only in autocommit** — measured, `Catalog only supports writes using
   autocommit: iceberg`, refusing DML *and* DDL inside an explicit transaction, same-table as well
   as cross-table — so although Trino has real `START TRANSACTION`/`COMMIT` syntax, that syntax is
   write-inert and a ledger write and its data write can never commit together. State it as
   permanent for that reason, not "not yet", and cite `phases/01-summary.md`'s measurement.
2. **`docs/specs/state.md` §Diagnostics** — no new code. Add one sentence after the table stating
   that on a backend realising no correctness structure (Spark, Trino) `MaintenanceStateDowngraded`
   is the *normal* outcome for every dependent cell and `DeclaredContractRequiresState` is reserved
   for the declarations whose semantics are themselves a statement about state.
3. **`docs/specs/multi_backend.md` §"Incremental & schema evolution per backend"** (line ~1140) —
   a paragraph for the `trino` target: it realises none of the five correctness structures, for the
   connector reason above; every cell that would need one takes the degradation contract's
   recompute-family downgrade carrying `MaintenanceStateDowngraded`, never a refusal; schema
   evolution is a separate axis Iceberg *does* support (forward reference only, phase 3 measures
   it). The existing §"Verification" paragraph's "not yet reachable on this target" wording for
   Trino maintenance is corrected to the permanent-absence-plus-downgrade framing.

## Tests

New file `crates/smelt-logical/tests/state_realisability_docs.rs` — red before the spec edits:

- `state_md_table_agrees_with_realisable_state_structures` — parses the markdown table under
  §"Which dialects realise which structure", maps each column to a `SqlDialect`, and asserts each
  cell's yes/no agrees with `realisable_state_structures(dialect)`. Every `SqlDialect` variant must
  have a column (a new dialect fails here rather than being silently undocumented). Red today:
  there is no Trino column.
- `trino_absence_is_stated_as_permanent_with_the_measured_reason` — the prose under the table names
  Trino, uses the autocommit-refusal reason, and does **not** describe Trino's cells as "not yet".
- `multi_backend_states_the_trino_state_posture` — §"Incremental & schema evolution per backend"
  names the `trino` target, states it realises no correctness structure, and names
  `MaintenanceStateDowngraded` as the consequence.
- `no_spec_text_calls_trino_maintenance_not_yet_reachable` — the corrected §Verification wording;
  guards against the "not yet" framing reappearing anywhere in `multi_backend.md` for Trino.

## Tasks

1. Write `state_realisability_docs.rs` with the four tests; confirm all four fail for the right
   reason (missing column / missing prose), not on a parse error.
2. Add the Trino column to `state.md`'s table and extend the reason paragraph with the measured
   autocommit finding, marked permanent.
3. Add the §Diagnostics sentence naming when each of the two existing codes applies on a
   no-structure backend.
4. Add the §"Incremental & schema evolution per backend" Trino paragraph in `multi_backend.md` and
   correct the §Verification "not yet reachable" sentence.
5. Re-run the new test; green.
6. Cross-check `crates/smelt-logical/tests/maintenance_availability/realisation.rs`'s `has_emitters`
   doc comment, which today says `20260913-trino-ledger` "revisits" Trino's absence — reword to
   record it as settled by measurement, with no behaviour change.
7. Flip the phase-2 row to `done` and write `phases/02-summary.md`.

## Verification

- `cargo test -p smelt-logical --test state_realisability_docs`
- `cargo test -p smelt-logical --test maintenance_availability`
- `cargo test -p smelt-core --test trino_docs_freshness`
- `bash .claude/scripts/verify-phase.sh`
- No baseline file bumped; no live tier needed (if the coordinator is down, this phase is still
  fully runnable — nothing here executes SQL).

## Commit message

`spec(state): Trino realises no correctness structure — permanent, measured, gated`
