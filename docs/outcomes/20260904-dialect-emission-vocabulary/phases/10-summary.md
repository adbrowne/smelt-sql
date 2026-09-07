# Phase 10 summary — invariant, refusal docs, issue trail

## Shipped

- `docs/specs/architecture.md` §Constraints item 14: names `Template`/`Conditional` verdicts,
  states the generic-printer / compile-path-settlement rule, and adds the build-time validation
  gate bullet (`registry_coverage`).
- `CLAUDE.md`'s Function-registry single-ownership bullet: mirrors the same two facts in one
  sentence each, plus a new gate line naming `template_emission`/`operand_conditional`/
  `registry_coverage`.
- `crates/smelt-runtime/tests/dialect_seam.rs`: doc-quote extraction factored into
  `assert_doc_quote_matches_live_diagnostic(marker, model_sql, backend)`; two new tests —
  `docs_quoted_template_modifier_refusal_matches_the_live_diagnostic` (DuckDB `DATE_SUB(DISTINCT
  d, INTERVAL 1 DAY)`) and `docs_quoted_operand_class_refusal_matches_the_live_diagnostic` (Spark
  `a // b` with an unresolvable operand). Both doc blocks were captured from actual compile-path
  output via a throwaway `eprintln!` test, then deleted.
- `docs-site/docs/reference/diagnostics.md`: two new `UnsupportedOnBackend` subsections — "A
  template's spelling cannot carry a modifier" and "A verdict that depends on operand type" —
  each with a pinned example block and a one-line Fix.
- `docs-site/docs/guide/targets.md`: a ~9-line "Per-operand-type lowering" note under §Cross-engine
  SQL compilation, linking to the new diagnostics subsection.
- `docs/ROADMAP.md`: the 2026-09-04 parallel track flipped to ✅ (September 6, 2026) with measured
  gap counts (`dialect_gaps_duckdb` 12→4, `dialect_gaps_spark` 27→4) and what remains named.
- GitHub: #177 and #178 commented + closed (their subject — a missing DuckDB/Spark emission
  verdict — is fully paid down). #173, #174, #179 commented, left open (BigQuery arms need the
  human-run sweep, out of scope for this outcome).

## Decisions

- Doc text was captured from live compiler output (a throwaway test, deleted after use), never
  hand-written — same measure-don't-assert discipline the rest of the outcome ran under.
- `//` on Spark's unresolvable-operand model uses an unschematised source table (`FROM t`) so the
  column types infer as `Unknown`/`Unresolved`, landing on the `otherwise` arm — matches the
  existing pattern already used by `a_model_using_floor_divide_fails_to_compile_for_bigquery`.
- Issue disposition split by subject, not by outcome: #177/#178 close because DuckDB/Spark have no
  missing emission verdict; #173/#174/#179 stay open because their BigQuery arms are unverified.

## For the next planner

- The outcome's success criteria are now all met or explicitly out-of-scope (BigQuery sweep); all
  10 phases and the outcome itself are `Status: done`.
- The BigQuery sweep (`scripts/bigquery-dialect-audit.sh`) is the one remaining piece of this
  programme and is explicitly human-run/billing — do not schedule it as an autonomous phase.
- Four `#175`/`#176` type-inference rows remain per dialect (DuckDB, Spark) — genuine inference
  bugs, tracked separately, not part of this outcome's scope.

## Gates

- `bash .claude/scripts/verify-phase.sh` — fmt PASS, clippy PASS, example_diagnostics PASS; `cargo
  test (workspace)` failed only on `smelt-core --test baseline
  materialize_tests::checkout_scratch_is_deleted_when_materialization_fails`, a pre-existing
  parallel-test scratch-dir race — confirmed unrelated via `cargo test -p smelt-core --test
  baseline -- --test-threads=1` (21/21 passed).
- `cargo test -p smelt-runtime --test dialect_seam` — 18/18 passed.
- `cargo test -p smelt-db --test dialect_audit` — 61/61 passed.
- `cargo test -p smelt-types --test registry_coverage` — 106/106 passed.
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test
  operand_conditional` — 11+8+6 passed.
- `git diff --stat` — only `CLAUDE.md`, `docs/ROADMAP.md`, `docs/specs/architecture.md`,
  `docs-site/docs/{guide/targets.md,reference/diagnostics.md}`, and the test file changed; no
  `.claude/dialect-gaps-baseline.txt` or `crates/*/src` emission-row change.
- `gh issue view 177/178 --json state` — both `CLOSED`.
