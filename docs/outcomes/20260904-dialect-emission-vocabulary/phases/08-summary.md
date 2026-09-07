# Phase 8 summary — the Spark schema-gap paydown (#178)

## Shipped

- `crates/smelt-types/src/signatures.rs`: explicit Spark emission verdicts for all 15
  `gap(...)` names #178 named — `AGE`/`DATE_SUB` (`Emission::Template("{0} - {1}")`),
  `TO_SECONDS` (`Template("make_interval(0, 0, 0, 0, 0, 0, {0})")`), `JSON_ARRAY_LENGTH`/
  `JSON_OBJECT_KEYS` (explicit `Emission::Native`), and `GLOB`/`JSON_ARRAY`/`JSON_OBJECT`/
  `JSON_CONTAINS`/`MAKE_TIME`/`MAKE_TIMESTAMPTZ`/`QUOTE_IDENT`/`QUOTE_LITERAL`/`TRUNCATE`/
  `GROUP_CONCAT` (`Emission::Unsupported`, each reason naming what's missing or the shape
  change a rename/template can't express).
- `crates/smelt-db/tests/dialect_audit/ledger.rs`: the 15 closed `gap` rows plus the 3
  redundant `gap_at(..., Position::Window)` rows (MEDIAN/PERCENTILE_CONT/PERCENTILE_DISC —
  already `Unsupported` at that position) deleted; one new `type_gap` (`DATE_SUB` on Spark,
  the bail-out clause the plan named in advance) and five `divergent` rows added (value/
  textual-representation differences: `AGE`, `TO_SECONDS`, `DATE_SUB`, `JSON_ARRAY_LENGTH`,
  `JSON_OBJECT_KEYS`).
- `crates/smelt-db/tests/dialect_audit/overrides.rs`: `JSON_ARRAY_LENGTH`/`JSON_OBJECT_KEYS`
  now probe with `j_json`, not a `Variadic(Any)`-derived numeric column — the actual root
  cause of the original "wants a JSON string, not a number" gap note.
- New tests: `registry_coverage::spark_schema_gap_names_have_an_explicit_verdict`,
  `::a_variadic_signature_still_rejects_a_template`; `unsupported_emission::spark_refuses_*`
  (10 names); `operand_conditional::{age,date_sub,to_seconds}_prints_the_spark_form`.
- `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_spark` 23 → 6, dated sign-off block
  naming every closed row and its measured verdict.
- `docs/reference/dialect-coverage.md` regenerated.

## Decisions

- Preferred `Emission::Native` over `Rename` for `JSON_ARRAY_LENGTH`/`JSON_OBJECT_KEYS`
  once live probing showed both already resolve correctly under their own name — the
  original gap was a probe-shape bug (wrong fixture column), not a real emission gap.
- Chose `Unsupported` over a novel "`Template` inside a `Conditional` arm" mechanism for
  arity-varying variadic functions (`GROUP_CONCAT`): `validate_conditional` doesn't
  currently re-validate a `SettledEmission::Template` arm's placeholders at all, so
  pioneering that combination here would add an unvalidated path — flagged for the next
  planner rather than done ad hoc.
- Treated interval/array textual-representation mismatches (`AGE`, `TO_SECONDS`,
  `JSON_OBJECT_KEYS`) as `divergent`, not gaps — same engine-formatting-only pattern as
  the existing `CONCAT`/`ARRAY_AGG` rows.

## For the next planner

- **Phase 9 is now unblocked and slightly bigger**: `DATE_SUB` on Spark joins `DATE_ADD`
  (both dialects) as a `type_gap` citing #176, all from the same missing-`SqlFunction`-
  enum-variant root cause phase 4 found on DuckDB.
- **Validation gap surfaced, not fixed**: `validate_conditional` never calls
  `validate_template` on a `SettledEmission::Template` arm, so a `Conditional` entry
  could carry a template with out-of-range placeholders today and it would only fail at
  print time (or not at all, silently substituting wrong text) rather than at registry
  construction. Worth a follow-up before anyone leans on Template-in-arm.
- Out of scope, not attempted: BigQuery legs of these same names (deferred to the
  outcome's declared BigQuery-sweep exclusion).

## Gates

- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/example_diagnostics green; the
  full `cargo test (workspace)` leg reported the pre-existing python-discovery
  test-isolation flake (`smelt-runtime`'s `python::tests::*` and
  `tests/combined_loop.rs`, different test names each run) — confirmed unrelated by
  re-running both with `--test-threads=1`: 216/216 and 5/5 pass.
- `SPARK_CONTAINER_ID=... cargo test -p smelt-db --test dialect_audit` — 61/61, including
  `schema_leg_spark`, `value_leg_spark`, `gap_count_ratchet`,
  `the_coverage_table_matches_the_registry`.
- `cargo test -p smelt-types --test registry_coverage` — 104/104.
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional --test unsupported_emission` — all green.
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity` — 21/21.
- `cargo test -p smelt-db --test integration -- registry_consistency` — 6/6.
- `git diff .claude/dialect-gaps-baseline.txt docs/reference/dialect-coverage.md` — both intended.
- `bash scripts/spark-down.sh` — done.
