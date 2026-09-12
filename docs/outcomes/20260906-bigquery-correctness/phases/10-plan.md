# Phase 10 plan — Close

## Objective

Close the outcome: prove every success criterion's evidence holds at today's HEAD, leave
the two artifacts criterion 4 names in a state a future reader can trust (the regenerated
coverage table, and the gap ratchets with a dated note saying *why* they held rather than
fell), report to issue #179 what this outcome actually verified and what it deliberately
did not, and add the one structural gate criterion 3 is still missing — a scan for the
defect class phases 1-3 fixed (a maintenance emitter taking a `dialect` parameter and
hardcoding DuckDB anyway), which today is held only by per-emitter unit tests.

## Spec delta

None. Nothing user-visible changes: the coverage doc regenerates byte-identical, the
ratchets hold, and the new gate is a test-only structural scan. Phase 9 already landed
this outcome's spec edit (`docs/specs/multi_backend.md` §"Known Divergences").

## Tests

New file `crates/smelt-logical/tests/maintenance_dialect_blindness.rs`:

1. `no_production_emitter_hardcodes_the_duckdb_dialect` — over every `.rs` under
   `crates/smelt-logical/src/maintenance/`, with `#[cfg(test)] mod tests` sections
   stripped, every surviving `MaintenanceDialect::DuckDb` occurrence must be part of a
   dispatch on a `dialect` binding (a `match`/`matches!` arm — the line contains `=>` or
   `matches!(dialect`), never a bare argument-position literal. Expected GREEN at HEAD;
   it is the regression gate, not a new fix.
2. `the_scan_flags_a_planted_argument_position_hardcode` — non-vacuity: the same
   classifier run over a synthetic source string containing
   `row_fingerprint_expr(cols, MaintenanceDialect::DuckDb)` reports exactly one offender.
3. `the_scan_ignores_a_hardcode_inside_a_test_module` — the stripper works: a planted
   offender placed after `#[cfg(test)]\nmod tests {` is not reported.

In `crates/smelt-cli/tests/github_activity_oracle.rs` (fixing the false positive phase 9's
summary flagged, which this phase's handoff edit would otherwise trip):

4. `the_handoff_scan_is_scoped_to_the_divergence_section` — a backtick-leading table row in
   a section *other* than the divergence table is not read as a registered-divergence
   claim. RED before `handoff_claimed_relations()` is scoped.
5. `the_handoff_scan_still_catches_a_stale_claim_in_its_own_section` — non-vacuity: a
   fabricated backtick-leading row *inside* the divergence section is still collected, so
   `findings_handoff_names_no_unknown_relation` keeps its teeth.

## Tasks

1. Write test 1's classifier and tests 1-3; confirm 1 is green at HEAD without touching
   `src/` (if it is red, the offender is a real phase-1/2 residue — fix it and say so).
2. Scope `handoff_claimed_relations()` to the handoff's divergence-table section (bounded
   by its heading and the next `##`/`###` heading); add tests 4-5; keep
   `every_registry_entry_is_named_in_the_findings_handoff` unchanged.
3. Append §"Close-out (2026-09-08)" to `docs/handoffs/2026-09-08-github-activity-findings.md`:
   one row per success criterion 1-7 with the artifact and the gate that holds it, and an
   explicit line naming what stayed unverified (the live BigQuery leg, the 42 no-verdict
   #179 entries) with the Out-of-scope rationale.
4. Append a dated hold note to `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_bigquery`
   stays at 42 because this outcome gave no BuiltinRegistry entry a new verdict (its fixes
   were in `smelt-logical`'s maintenance emitters), so criterion 4's "fall or hold" is
   satisfied by holding. Counts must NOT change — `gap_count_ratchet` asserts exact match.
   Confirm `.claude/parser-gaps-baseline.txt` still reads `duckdb_seed_gaps 0`, untouched.
5. Run `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit
   the_coverage_table_matches_the_registry` and confirm `git status` shows
   `docs/reference/dialect-coverage.md` unchanged; record that it regenerated clean.
6. Post a **comment** on issue #179 (`gh issue comment 179`) — do not close it — naming:
   the maintenance-emitter dialect fixes this outcome landed (phases 1-3), the offline
   gates now standing (`googlesql_render`, the new blindness scan, `modulo_lowering`,
   `power_lowering`), and that the 42 no-verdict entries are untouched and still owned by
   #179 because no spine model reached them (criterion 2).
7. Run every gate in criterion 7 and record verbatim pass counts in the summary.
8. Append the dated phase-10 entry to the outcome's Decision log; flip row 10 to `done`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_dialect_blindness`
- `cargo test -p smelt-cli --test github_activity_oracle`
- `cargo test -p smelt-db --test dialect_audit` (includes `gap_count_ratchet`,
  `the_coverage_table_matches_the_registry`, `every_entry_and_dialect_appears_in_the_table`)
- `cargo test -p smelt-dialect --test emission_ownership`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance`
- `cargo test -p smelt-maintenance-testkit --test googlesql_render`
- `bash .claude/scripts/large-file-check.sh`

The BigQuery **value** leg (`scripts/bigquery-dialect-audit.sh`) is not run: it needs a
live warehouse. Its absence is already a named, dated debt in
`docs/specs/multi_backend.md` §"Known Divergences" (phase 9) — record it again in the
close-out section rather than skipping it silently. This does not warrant
`<<PHASE_BLOCKED>>`: no task above needs it.

## Commit message

`outcome(bigquery-correctness): close — gate dialect blindness, scope the handoff scan, hold the ratchets`
