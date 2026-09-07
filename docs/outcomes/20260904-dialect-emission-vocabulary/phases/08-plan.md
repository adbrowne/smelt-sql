# Phase 8 plan — the Spark schema-gap paydown (#178)

## Objective

Close every surviving Spark **schema-leg** ledger row: the sixteen `gap(...)` names and the three
redundant `gap_at(..., Position::Window)` running-window rows. Each closure is a `Rename`,
`Template`, conditional arm, or `Unsupported { reason }` **measured against a live Spark**, never
read from documentation. Advances criterion 5 (`dialect_gaps_spark`) and criterion 9 (the live
Spark legs of `dialect_audit`).

## Spec delta

None. `docs/specs/multi_backend.md` §"Operator lowering" and §"Cross-engine emission audit"
already state the verdict vocabulary and the measured-not-documented rule normatively; this phase
adds registry rows under existing rules. If a measurement contradicts a §Known Divergences
sentence, correct that sentence in the same commit.

## Tests

- `registry_coverage::spark_schema_gap_names_have_an_explicit_verdict` — for each of the sixteen
  names, `BuiltinRegistry::resolve(name)` yields an emission entry for `DialectId::SparkSql`
  (not the implicit `Native` default). Red before the registry edits.
- `registry_coverage::a_variadic_signature_still_rejects_a_template` — guards the constraint that
  shapes this phase: `any_args()` rows can only take `Rename`/`Unsupported`/`Conditional`.
- `unsupported_emission::spark_refuses_<name>_with_a_named_reason` — one test per name that lands
  `Unsupported`: compiling the call for Spark fails with `UnsupportedOnBackend` carrying the
  registry's reason text.
- `operand_conditional::<name>_prints_the_spark_form` — one per name that lands `Rename`,
  `Template` or a conditional arm: asserts the printed Spark SQL, pinned.
- `dialect_audit::schema_leg_spark` / `::value_leg_spark` (live) — the measurement itself; every
  probe for a closed row must pass or be exempted by its own declared `Unsupported`.
- `dialect_audit::gap_count_ratchet` — `dialect_gaps_spark` equals the new baseline exactly.
- `dialect_audit::a_ledger_row_the_engine_now_accepts_is_reported_stale` — unchanged; confirms a
  deleted row cannot be re-added silently.

## Tasks

1. `bash scripts/spark-up.sh` **from this worktree** (the container is a singleton bound to
   whichever worktree last started it), then `source scripts/spark-env.sh`. If the server will not
   start, stop and block the phase — never author a verdict from documentation.
2. Run `SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) cargo test -p smelt-db --test
   dialect_audit` to confirm a clean 23-gap baseline before any edit.
3. For each of the sixteen names, probe the live Spark directly (spark-sql over the fixture) for
   the narrowest verdict its signature admits, in this preference order: `Rename` (same shape,
   different name) → `Template` (fixed-arity rows only — `AGE`, `TO_SECONDS`, and any other with a
   fixed `vec![...]` param list) → `Conditional` arm → `Unsupported { reason }`. Record the
   measured evidence in the registry row's doc comment, dated.
4. Land those registry edits in `crates/smelt-types/src/signatures.rs`; `GLOB` is
   `SyntaxForm::Infix`, so verify its verdict prints through the operator emission site, not the
   function-call one.
5. Delete the three `gap_at(..., Position::Window)` Spark rows for `MEDIAN`, `PERCENTILE_CONT`,
   `PERCENTILE_DISC` — each is already `Emission::Unsupported` at that position in the registry,
   so the row is redundant (same finding phase 4 acted on for DuckDB). Confirm the audit's
   declared-unsupported exemption covers them rather than assuming it.
6. Delete the sixteen closed `gap(...)` rows from `crates/smelt-db/tests/dialect_audit/ledger.rs`.
7. Re-run the live audit. **Bail-out clause** (the one phase 4 used): if `DATE_ADD`/`DATE_SUB`'s
   Spark *type* legs now fail for the reason recorded in the DuckDB ledger comment, add them as
   `type_gap` rows citing `#176`, leaving `dialect_gaps_spark` at 6; phase 9 closes that family on
   both dialects. Do not paper over it, and do not chase it here.
8. Update `.claude/dialect-gaps-baseline.txt` with the new `dialect_gaps_spark` count and a dated
   sign-off block naming every closed row and its measured verdict; leave `dialect_gaps_duckdb`
   and `dialect_gaps_bigquery` untouched.
9. Regenerate `docs/reference/dialect-coverage.md` and confirm the doc-sync gate is green.

## Verification

- `bash .claude/scripts/verify-phase.sh` (the `smelt-runtime` python-discovery flake is
  pre-existing and unrelated — confirm any failure is that one by re-running with
  `--test-threads=1`, and say so explicitly in the summary).
- Live: `SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) cargo test -p smelt-db --test
  dialect_audit` — all legs green including `schema_leg_spark`, `value_leg_spark`,
  `gap_count_ratchet`, `every_conditional_arm_is_covered_by_a_probe`.
- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional
  --test unsupported_emission`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance
  --test restructure_multiplicity`
- `cargo test -p smelt-db --test integration -- registry_consistency`
- `git diff .claude/dialect-gaps-baseline.txt docs/reference/dialect-coverage.md` — both intended.
- `bash scripts/spark-down.sh` when finished.

## Commit message

`feat(dialect): close the Spark schema-gap ledger with measured emission verdicts`
