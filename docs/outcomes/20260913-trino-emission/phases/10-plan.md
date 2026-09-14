# Phase 10 plan — close: the published table and the spec tell the same truth

## Objective

Close the outcome by making every *published* statement about Trino emission match what the
gates now enforce. Advances criterion 6 (the published table tells the truth), criterion 1's
final leg (the implicit-`Native` divergence is retired because a gate closes it, not because
someone remembers), and criterion 12 (all gates green, no ratchet lowered). No new emission
verdicts, no new probes — this phase only removes stale claims and proves the standing gates.

## Spec delta (first)

`docs/specs/multi_backend.md`:

1. **§"Known Divergences / Open Questions" — delete the "Every built-in is implicitly `Native`
   on Trino" entry** (line ~1558). It is closed: `Signature::stated_emission_at` separates a
   stated verdict from the implicit default, `dialect_audit`'s `census.rs` classifies every
   `(entry, position)` pair as `Stated`/`Verified`/`Gap`/`Unverified` and fails on the last,
   and `.claude/trino-emission-census.txt` reached zero and was deleted. Nothing replaces it in
   §Known Divergences — the standing rule already lives in §"Cross-engine emission audit".
2. **§"Known Divergences" — re-verify the "A Trino `array(...)` result column does not decode
   to Arrow yet" entry** against the code phase 9 changed. Phase 9 taught
   `trino_type_to_arrow` the `array(...)` *type signature*; whether the result-page *cell*
   decoder in `crates/smelt-backend-trino/src/` now produces an Arrow list value is the open
   question. Land whichever is true: delete the entry if a live projection of an array column
   reads back, or narrow its wording to name the cell decoder specifically (not the type map)
   so it no longer reads as a gap phase 9 already fixed. A test decides this, not a reading.
3. **§References — drop the stale forward reference.** `crates/smelt-backend-trino/` is listed
   as "(forward reference — lands in `docs/outcomes/20260913-trino-target-spine`)"; that
   outcome is `done`. State it as a plain code reference.

## Tests (red-green)

- `array_result_column_decodes_to_arrow` (in `crates/smelt-backend-trino/`, live-gated like the
  crate's other coordinator tests) — projects an `array(...)`-typed column through the real
  statement client and asserts an Arrow list value, not an error. This is the test that decides
  spec delta 2; if it stays red, it is registered as the *narrowed* divergence, not deleted.
- `the_coverage_table_matches_the_registry` (existing, `dialect_audit/coverage_table.rs`) —
  must be green *without* `SMELT_REGEN_DOCS`, proving the regenerated table is committed.
- `census::*` + `registry_totality::*` (existing) — assert zero `Unverified` Trino pairs with
  no census file present; confirm the no-census path is the one being exercised.

## Tasks

1. Write `array_result_column_decodes_to_arrow` red-first; run it live and record the verdict.
2. Apply spec deltas 1–3 to `docs/specs/multi_backend.md` per that verdict.
3. Re-read every `#209` Trino row in `crates/smelt-db/tests/dialect_audit/ledger.rs` for a
   reason of the form "type signature the client does not yet decode" — phase 9's carry-forward.
   Four such rows were already removed there; confirm by a live schema sweep that none of the
   remaining 60-odd are stale (the two-sided ledger fails on a row the engine now accepts, so a
   green live `dialect_audit` *is* the proof — state that in the summary rather than re-reading
   by eye alone).
4. Regenerate the published table: `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test
   dialect_audit the_coverage_table_matches_the_registry`, then commit whatever it changes.
5. Check `docs-site/docs/guide/targets.md`'s Trino section for any statement this outcome made
   false (an unqualified "every built-in works", a missing pointer to
   `docs/reference/dialect-coverage.md`); fix or add the pointer if so.
6. Run the full live sweep and `verify-phase.sh`; record the evidence lines the outcome's
   Decision log will cite when it is marked `done`.
7. Write `phases/10-summary.md` including a criterion-by-criterion (1–12) evidence table, since
   the next planner's job is to judge the Success criteria and close the outcome.

## Verification

- `bash .claude/scripts/verify-phase.sh` — must be ALL GREEN.
- Live (`bash scripts/trino-up.sh` + `source scripts/trino-env.sh`), each read in the
  foreground and **never skipped green** — if the coordinator is unreachable, emit
  `<<PHASE_BLOCKED>>` rather than reporting a pass:
  - `cargo test -p smelt-db --test dialect_audit` (both legs, ledger two-sidedness, gap
    ratchet, doc-sync gate)
  - `cargo test -p smelt-backend-trino`
  - `cargo test -p smelt-db --test type_property_tests`
- `cargo test -p smelt-dialect --test emission_ownership`,
  `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance`.
- `.claude/dialect-gaps-baseline.txt`, `registry-migration-baseline.txt`,
  `parser-gaps-baseline.txt`, `hardening-baseline.txt`: unchanged or *tightened* only; any
  change carries a sign-off note.

## Commit message

`docs(dialect): retire the Trino implicit-Native divergence and republish the coverage table`
