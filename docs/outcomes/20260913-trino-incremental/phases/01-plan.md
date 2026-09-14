# Phase 1 plan — Characterise Iceberg `MERGE` by execution

## Objective

Establish, by running each clause form against the live Trino/Iceberg tier, exactly which `MERGE`
shapes the connector accepts — and in particular whether the whole-row `UPDATE SET *` / `INSERT *`
spellings work or whether Trino needs BigQuery's column-by-column rendering. Advances criterion 1
directly, and de-risks criteria 2, 3 and 5 by fixing the emitter shape phases 3 and 5 must target
before any of them is written. Every `✗` leaves the coordinator's own error text in the decision
log, so a later phase refuses by name against a measured message rather than a guess.

## Spec delta

None planned. Phase 2 owns the prose in `docs/specs/multi_backend.md` §"Whole-row MERGE" /
§"Column-scoped merge and conditional-write capabilities" / §"Incremental & schema evolution per
backend". This phase touches the spec **only if a measurement contradicts a matrix cell**: if
`supports_merge`, `supports_column_scoped_merge` or `supports_merge_not_matched_by_source` measures
differently from the §Surface capability matrix or `BackendCapabilities::trino_iceberg()`, both are
corrected in this commit (criterion 1's "confirmed or corrected in the same commit").

## Tests

New file `crates/smelt-backend-trino/tests/merge_clause_forms.rs`, following
`capability_probes.rs`'s live-env harness (`live_env_or_skip`, unique per-run schema, `drop_schema`)
— skip-green when `SMELT_TRINO_URL` is unset, which `compat.yml`'s existing `grep -qi skipping`
guard turns into a CI failure when the tier is up.

1. `merge_whole_row_update_set_star` — measures whether `WHEN MATCHED THEN UPDATE SET *` is accepted;
   asserts the recorded verdict, not a hoped-for one.
2. `merge_whole_row_insert_star` — same for `WHEN NOT MATCHED THEN INSERT *` (and, if rejected, which
   of `INSERT ROW` / explicit column list Trino wants instead).
3. `merge_named_column_update_set_computes_the_same_rows_as_the_star_form` — value leg: the
   column-by-column fallback and the star form (whichever exist) leave byte-identical table contents.
4. `merge_when_matched_conditional_and_guard` — `WHEN MATCHED AND <pred> THEN UPDATE ...` accepted,
   and the guard actually selects rows (value leg, not acceptance only).
5. `merge_when_matched_then_delete` — whether the delete arm exists; needed by the merge-less
   conditional-write and delete-and-insert routes later.
6. `merge_multiple_when_clauses_are_first_match_wins` — two ordered matched arms; asserts the
   resulting rows show first-match-wins ordering.
7. `merge_source_may_be_a_subquery_over_a_staged_relation` — the source shape T3's staged relation
   actually presents (`USING (SELECT ... FROM <staged>) s`), not a values list.
8. `merge_not_matched_by_source_error_text_is_stable` — asserts the form fails and that the
   coordinator's message matches the recorded substring, so a later by-name refusal is anchored to
   measured text.
9. `every_measured_clause_form_is_recorded_in_the_decision_log` — reads
   `docs/outcomes/20260913-trino-incremental/outcome.md` and asserts each clause form named by a test
   above appears in the decision log entry, so a measurement cannot land undocumented.

## Tasks

1. `bash scripts/trino-up.sh && source scripts/trino-env.sh`; confirm the coordinator answers. If it
   cannot be reached, emit `<<PHASE_BLOCKED>>` — never skip green.
2. Add `merge_clause_forms.rs` with the shared live-env harness (lift, don't copy-paste-and-diverge:
   if the harness is worth sharing, put it in a `tests/common/` module both files use).
3. Write tests 1–8 red-green, one clause form at a time, capturing each failure's verbatim error text
   as you go.
4. Run the whole file against the live tier; record every verdict and every measured error string.
5. Append the dated measurement table + quoted error text to the outcome's `## Decision log`, then add
   test 9 to hold it there.
6. Confirm the three merge flags against the measurements; correct `BackendCapabilities::trino_iceberg()`
   and the §Surface matrix in this commit if any disagrees (expected: no change).
7. Write `phases/01-summary.md`, naming explicitly which emitter shape phase 3 must target.

## Verification

- `bash scripts/trino-up.sh && source scripts/trino-env.sh`
- `cargo test -p smelt-backend-trino --test merge_clause_forms` — all pass, none skipped
- `cargo test -p smelt-backend-trino --quiet 2>&1 | tail -40` — no `Skipping` line with the tier up
  (the condition `compat.yml` enforces)
- `cargo test -p smelt-core --test trino_docs_freshness` — unchanged
- `bash .claude/scripts/large-file-check.sh`
- `bash .claude/scripts/verify-phase.sh`
- `bash scripts/trino-down.sh`

## Commit message

`test(trino): characterise Iceberg MERGE clause forms by execution against the live tier`
