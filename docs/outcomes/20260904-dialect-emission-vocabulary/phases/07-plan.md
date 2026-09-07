# Phase 7 plan — Spark's conditional and template verdicts

## Objective

Land the first *production* `Emission::Conditional` rows and the first Spark `Emission::Template`
row: `LOG` by arity, `DAYOFWEEK` as a shift template, `//` per operand class, and `TRUNC`/`TO_JSON`
by the class of their first argument. This advances criterion 5 (`dialect_gaps_spark` 27 → 23, each
closure live-verified) and criterion 4 (the settlement path proven on real registry entries rather
than synthetic ones), and turns phase 6's `every_conditional_arm_is_covered_by_a_probe` and the
arm-keyed ledger from green-but-vacuous into real checks.

## Spec delta

`docs/specs/multi_backend.md` §Known Divergences (the bullet at ~line 963 and the `DAYOFWEEK`
bullet at ~line 970). The normative rules already exist — §"Operand-conditional verdicts" describes
`//`'s per-class arms (l. 238–244), §"Template emission" describes the non-call template
`DAYOFWEEK({0}) - 1` (l. 329), and l. 359–363 already state `LOG`'s arity split and Spark's
temporal-only `TRUNC` / composite-only `TO_JSON`. The edit is subtractive: strike from the
Known-Divergences bullet the clauses this phase closes (`//` refused wholesale on Spark, `LOG`
registered at one arity, Spark's `TRUNC`/`TO_JSON` carrying no class-scoped arm) and delete the
`DAYOFWEEK` bullet, leaving the BigQuery clauses (#173, GoogleSQL's reversed `LOG(base, x)`)
standing — BigQuery is out of scope for this outcome.

## Tests

Red-green, each red before its row exists:

1. `smelt-types` `registry_coverage::log_settles_by_arity_on_spark` — `settle_at` on the real `LOG`
   entry: `CallFacts` of arity 1 → `Rename("LOG10")`, arity 2 → `Native`; DuckDB unchanged (`Native`
   at both arities).
2. `smelt-dialect` `operand_conditional::dayofweek_prints_the_shift_template_on_spark` — byte-pinned
   printed SQL for `SELECT DAYOFWEEK(d) FROM t` on Spark; asserts the non-call template's whole
   output is parenthesised (phase 2's rule) and that DuckDB's output is unchanged.
3. `smelt-types` `registry_coverage::intdiv_settles_per_operand_class_on_spark` — `//` with
   (Integral, Integral) → the integral template; (Floating, Floating) and (Decimal, Decimal) → the
   plain-division template; (Unresolved, _) → `Unsupported`. One assertion per class pair.
4. `smelt-dialect` `operand_conditional::trunc_and_to_json_settle_by_first_argument_class` —
   `TRUNC` arg0 `Temporal` → `Native`, non-temporal → `Unsupported`; `TO_JSON` arg0 `Composite` →
   `Native`, scalar → `Unsupported`. Reasons name the argument class.
5. `smelt-runtime` `dialect_seam::intdiv_over_typed_integer_columns_compiles_on_spark` — a model
   with `a // b` over two `INTEGER` columns compiles for Spark (previously `UnsupportedOnBackend`);
   the existing unresolvable-operand refusal test stays green unchanged.
6. `smelt-db` `dialect_audit::the_conditional_arm_gate_is_no_longer_vacuous` — asserts the registry
   holds at least one `Emission::Conditional` entry, so
   `every_conditional_arm_is_covered_by_a_probe` cannot silently pass on an empty set again.
7. Live: `dialect_audit` `schema_leg_spark` + `value_leg_spark` green with `SPARK_CONTAINER_ID`
   set, and `gap_count_ratchet` at `dialect_gaps_spark 23`.

## Tasks

1. `bash scripts/spark-up.sh`; `source scripts/spark-env.sh`;
   `export SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark)`. **If the server does not come up,
   stop**: flip phase 7 to `blocked`, append to `## Blocked`, and emit the blocked sentinel. Never
   author a verdict from documentation. (Note the singleton-container trap: the container binds to
   whichever worktree last ran `spark-up.sh` — run it from *this* worktree.)
2. Before writing any registry row, measure each candidate on the live engine and against DuckDB
   1.5.4 as the reference: `LOG(x)` vs `LOG10(x)`, `LOG(b, x)`; `DAYOFWEEK(d) - 1`; `a // b` for
   integral, floating and decimal operands (DuckDB truncates toward zero for integers and degrades
   to plain division for floats — the Spark arm must reproduce *that*, not Spark's own default);
   `TRUNC` with a numeric and with a date argument; `TO_JSON` with a scalar and with a struct.
   Record the measured spellings in the phase summary.
3. Add the Spark rows in `crates/smelt-types/src/signatures.rs`. Heed phase 6's wiring trap: an arm
   set only takes effect if the signature declares `Emission::Conditional(ARMS)` in its own
   `.with_emission(...)` table for `(SparkSql, Position::Any)`. Every arm list ends in a
   guard-free `otherwise` arm.
4. `//`: replace Spark's wholesale `Emission::Unsupported` with the arm list, keeping the existing
   reason text as the `otherwise` arm's. Leave BigQuery's row untouched.
5. Delete the now-closed ledger rows in `crates/smelt-db/tests/dialect_audit/ledger.rs`:
   `value_gap("LOG", …)`, `value_gap("DAYOFWEEK", …)`, `gap("TRUNC", …)`, `gap("TO_JSON", …)`. A
   declared-`Unsupported` arm is exempting, so a refused arm closes its row rather than leaving an
   arm-scoped `Gap`. If a measurement contradicts a candidate lowering, keep the row, re-scope it
   with `arm`, and say so in the summary rather than forcing the count.
6. Regenerate the coverage doc: `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit
   the_coverage_table_matches_the_registry`. Conditional cells must render as their arm sets.
7. Edit `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_spark 27` → `23`, with a dated comment
   naming each of the four closures, its verdict kind, and the sign-off line (`sign-off: Andrew
   Browne`), in the style of phase 4's entry.
8. Make the spec delta above.
9. `bash scripts/spark-down.sh`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `SPARK_CONTAINER_ID=$(docker ps -qf name=smelt-spark) cargo test -p smelt-db --test dialect_audit`
  (the Spark legs skip green when the var is unset — a run without it proves nothing)
- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-dialect --test emission_ownership --test operand_conditional`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity`
- `cargo test -p smelt-db --test integration registry_consistency`
- `git diff .claude/dialect-gaps-baseline.txt docs/reference/dialect-coverage.md` — both intended
  and reviewer-legible; `dialect_gaps_duckdb` and `dialect_gaps_bigquery` unchanged.

## Commit message

`feat(dialect): settle LOG, DAYOFWEEK, //, TRUNC and TO_JSON on Spark by arity and operand class`
