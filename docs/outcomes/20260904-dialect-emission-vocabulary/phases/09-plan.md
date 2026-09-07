# Phase 9 plan — close the `DATE_ADD`/`DATE_SUB` type-leg family

## Objective

Land criterion 5 on both live engines: `dialect_gaps_duckdb` 6 → 4 and `dialect_gaps_spark`
6 → 4, leaving only the four `#175`/`#176` rows (`FIRST`, `LAST`, `EXPLODE`, `UNNEST`) on each
dialect. The root cause is a misclassification, not a lowering hole: `DATE_ADD`/`DATE_SUB` are
`SyntaxForm::Special` registry rows — exempt from the callable-function surface — yet the audit
probes them as calls, real smelt SQL parses them as calls, and `infer_function_type` bails at
`SqlFunction::from_name(...)?` before the registry is ever consulted, so both infer
`Unknown(Dynamic)`. The phase also closes the criterion-2 hole phase 8 surfaced:
`validate_conditional` never validates a `SettledEmission::Template` arm's placeholders.

## Spec delta (first)

`docs/specs/architecture.md` §Constraints item 14, **Consistency gate** bullet: the parenthetical
listing dedicated-syntax exemptions currently names `DATE_ADD`/`DATE_SUB` alongside `CAST`,
`LIKE`, `IN`, … . Remove those two names — they are ordinary two-argument calls on the callable
surface, and the row that typed them was miscategorised. The sentence's mechanism (the exemption
is derived from `SyntaxForm`, not a named list) is unchanged.

`docs/specs/multi_backend.md` §"Template emission": add one sentence stating that a template used
as a `Conditional` arm's verdict is validated by the same rules as a top-level template, at
registry construction.

## Tests (red first)

- `smelt-types registry_coverage::date_add_and_date_sub_are_ordinary_calls` — both signatures
  are `SyntaxForm::Call`, so the consistency gate's `SyntaxForm` exemption no longer covers them.
- `smelt-db integration::registry_inference::date_add_infers_timestamp` — `SELECT DATE_ADD(d,
  INTERVAL 1 DAY)` over a `Date` column infers `Timestamp`, not `Unknown`.
- `smelt-db integration::registry_inference::date_sub_infers_timestamp` — same for `DATE_SUB`.
- `smelt-dialect template_emission::date_add_prints_the_spark_form` — `DATE_ADD(a, b)` prints the
  measured Spark spelling; DuckDB output unchanged (`DATE_ADD(a, b)`).
- `smelt-dialect template_emission::date_sub_spark_form_matches_smelt_return_type` — the Spark
  template's printed text is the one whose engine-reported type equals smelt's declared
  `Timestamp`.
- `smelt-types registry_coverage::a_conditional_arm_template_is_validated` — a synthetic
  `Conditional` whose arm verdict is `SettledEmission::Template("{9}")` fails to build.
- `smelt-db dialect_audit::gap_count_ratchet` (live, both engines) — 4 and 4.

## Tasks

1. Land the two spec edits above.
2. Drop `.with_syntax_form(SyntaxForm::Special)` from `DATE_ADD`/`DATE_SUB` in
   `crates/smelt-types/src/signatures/builtins/extended_temporal.rs`; keep the `Timestamp` return
   type phase 4 corrected.
3. Add `SqlFunction` variants for both (scalar classification, matching the registry's `ExprKind`)
   and add both names to `REGISTRY_MIGRATED` in `type_inference/function_call.rs`, so the
   `legacy_match_ratchet` count is unchanged (they arrive already migrated).
4. Verify the DuckDB legs against the pinned DuckDB 1.5.4 library (not the ambient CLI — phase 4's
   trap): `DATE_ADD` stays `Native`, `DATE_SUB` keeps its `Template("{0} - {1}")`, and both type
   legs now report `TIMESTAMP`. Delete the two DuckDB `type_gap` rows.
5. Bring up Spark (`bash scripts/spark-up.sh`; `source scripts/spark-env.sh`). **Block, never
   fake, if the server cannot start.** Measure the spelling for both names whose engine-reported
   type is `TIMESTAMP`: the expected answer is `Emission::Template("CAST({0} + {1} AS TIMESTAMP)")`
   / `("CAST({0} - {1} AS TIMESTAMP)")`, since bare `DATE ± INTERVAL` stays `DATE` on Spark while
   smelt's declared semantics (and DuckDB) widen to `TIMESTAMP`. Register whatever is measured.
6. Delete the Spark `gap("DATE_ADD", …)` and `type_gap("DATE_SUB", …)` rows, and the now-stale
   Spark `divergent("DATE_SUB", …)` value row (the cast closes the value difference too — the
   ledger's two-sided check will name it if it does not).
7. Call `validate_template` from `validate_conditional` for every `SettledEmission::Template` arm
   verdict, threading the arm index into the error.
8. Update `.claude/dialect-gaps-baseline.txt` to `dialect_gaps_duckdb 4` / `dialect_gaps_spark 4`
   with a dated sign-off block naming each closed row and its measured verdict.
9. Regenerate `docs/reference/dialect-coverage.md`; re-key any `.claude/unknown-census.toml`
   line-number entries shifted by the `signatures/` edits (phases 3 and 4 both hit this).
10. `bash scripts/spark-down.sh`.

## Contingency (bounded, not deferred)

If no measured Spark spelling makes the engine report the type smelt declares, the difference is a
dialect-dependent return type smelt's dialect-independent inference cannot express. In that case
record the pair as a **type-leg `Divergent`** row (a new `type_divergent` helper: `Leg::Type`,
`Verdict::Divergent`), not a `Gap` — criterion 5 still lands, since Divergent rows do not ratchet —
and state the measurement and the reasoning in the phase summary and Decision log. Do not reclassify
without first attempting the cast.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test integration -- registry_consistency registry_inference`
- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-dialect --test template_emission --test emission_ownership --test operand_conditional --test unsupported_emission`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity`
- `cargo test -p smelt-db --test type_property_tests`
- `SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) cargo test -p smelt-db --test dialect_audit`
  (all legs, both engines, including `gap_count_ratchet` and `the_coverage_table_matches_the_registry`)
- `git diff .claude/dialect-gaps-baseline.txt docs/reference/dialect-coverage.md` — both intended.

## Commit message

`feat(registry): make DATE_ADD/DATE_SUB callable and close their type-leg gaps`
