# Phase 10 summary — close

## Shipped

- `docs/specs/multi_backend.md`: deleted the "Every built-in is implicitly `Native` on Trino"
  divergence entry (the gate that closes it — `census.rs`'s `Coverage::Unverified` fail —
  already exists and `.claude/trino-emission-census.txt` is gone).
- `docs/specs/multi_backend.md`: narrowed the array-decode divergence entry to name the *cell*
  decoder (`build_column` in `crates/smelt-backend-trino/src/arrow_convert.rs`) specifically,
  now that `trino_type_to_arrow` (phase 9) correctly maps the `array(...)` type signature —
  a live test proved the schema-level fix didn't also fix cell decoding.
- `docs/specs/multi_backend.md` §References: dropped the stale "(forward reference — lands in
  `docs/outcomes/20260913-trino-target-spine`)" annotation on `crates/smelt-backend-trino/`
  (that outcome is `done`).
- `crates/smelt-backend-trino/tests/backend_live.rs`: new live test
  `array_result_column_decodes_to_arrow`, which decided the spec-delta verdict above. It
  documents the exact failure (`build_column` has no `DataType::List` builder arm) via an
  `expect_err` assertion, rather than leaving a permanently-red integration test in the suite.

## Decisions

- The array-decode gap is real and unfixed; per the plan's explicit scope ("no new emission
  verdicts, no new probes"), I did not implement `DataType::List` decoding in `build_column`.
  The narrowed spec entry and the new regression test are the record of it.
- Converted the "red-first" test into a passing test that asserts the specific error message
  once the verdict was captured, rather than shipping a permanently-failing test — the plan's
  TDD framing was for *deciding* the spec wording, not for landing new failing coverage as a
  standing gate. The test carries an explicit comment: delete it and close the divergence entry
  if `build_column` ever gains a `List` arm.
- `docs-site/docs/guide/targets.md`'s Trino section already qualifies its statements correctly
  (both known gaps stated, pointer to §Known Divergences present) and matches the pattern used
  by every other target's section (none of them link `docs/reference/dialect-coverage.md`
  either) — no edit needed there.
- `docs/reference/dialect-coverage.md` needed no regeneration: `SMELT_REGEN_DOCS=1` produced a
  byte-identical file, confirming the committed table already matches the registry.

## For the next planner

- The array-cell-decode gap (`build_column` List arm) and the two capability-flag gaps named
  in `docs-site/docs/guide/targets.md` (`supports_native_ivm`/no maintenance dialect) are the
  concrete residue for future Trino work — the maintenance-dialect gap is explicitly T4's
  (`20260913-trino-incremental`), and array decoding has no owning outcome yet if it's wanted.
- `capability_probes.rs` has pre-existing multi-threaded flakiness against the live tier
  (`language_properties_are_measured`, `probe_supports_merge` intermittently fail
  `ensure_schema` under `cargo test`'s default parallelism — namespace-creation contention
  against the Iceberg REST catalog/MinIO). Passes clean at `--test-threads=1`. Not this
  outcome's target; flagging as a candidate fix for whichever outcome touches that test file
  next (probably `20260913-trino-incremental` or `20260913-trino-dogfood`).
- All 12 success criteria are satisfied (criteria 1-11 landed in phases 1-9; this phase's job
  was purely closing the stale documentation claims criterion 6 and 12 require). The outcome
  is ready to be marked `done` by the next planner.

### Criterion evidence (1-12)

| # | Criterion | Evidence |
|---|-----------|----------|
| 1 | Implicit-Native hole closed by a gate | `census::tests::no_dialect_has_unverified_pairs` green; census file deleted; spec entry removed this phase |
| 2 | Known divergences carry explicit verdicts | `registry_totality::*` green (phases 2-3) |
| 3 | PIVOT decided | phase 4, `unpivot_is_refused_for_trino_on_the_compile_path` green |
| 4 | Live audit, both directions | `trino::schema_leg_trino`, `trino::value_leg_trino` green |
| 5 | Two-sided ledger for Trino | `ledger_gates::*` green; `.claude/dialect-gaps-baseline.txt` unchanged this phase |
| 6 | Published table tells the truth | `coverage_table::the_coverage_table_matches_the_registry` green without `SMELT_REGEN_DOCS`; regen produced no diff |
| 7 | Ownership not diluted | `cargo test -p smelt-dialect --test emission_ownership` — 11/11 green |
| 8 | Compile-time refusal | `cargo test -p smelt-runtime --test dialect_seam` — 25/25 green, incl. Trino function-body refusals |
| 9 | Projection source-derived, 4 dialects | `projection_dialect_invariance::output_columns_and_cast_wrap_names_are_byte_identical_across_backends` green |
| 10 | Statement-level restructure on Trino | phase 7, `trino::trino_arg_max_window_agrees_with_duckdb_native` green |
| 11 | Types conformant or registered | `cargo test -p smelt-db --test type_property_tests` — 93/93 green, incl. `trino_coverage_floor_tests` |
| 12 | Gates green, no ratchet lowered | `verify-phase.sh` ALL GREEN; all four baseline files unchanged (`git status --short` empty) |

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN.
- Live (`scripts/trino-up.sh` + `scripts/trino-env.sh`):
  - `cargo test -p smelt-db --test dialect_audit` — 73/73 passed.
  - `cargo test -p smelt-backend-trino` — 25 lib + 12 `backend_live` passed; `capability_probes`
    27/27 passed at `--test-threads=1` (2 pre-existing flaky under default parallelism, see
    above — not this phase's target).
  - `cargo test -p smelt-db --test type_property_tests` — 93/93 passed.
- `cargo test -p smelt-dialect --test emission_ownership` — 11/11 passed.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance` —
  25/25 + 4/4 passed.
- `.claude/dialect-gaps-baseline.txt`, `registry-migration-baseline.txt`,
  `parser-gaps-baseline.txt`, `hardening-baseline.txt` — unchanged.
