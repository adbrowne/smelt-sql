# Phase 3 summary — modifier refusal, interpreter ownership gates, `template` legend

## Shipped

- `template_unsupported_modifier` in `crates/smelt-dialect/src/emission_check.rs`: a pure
  detector over a `FUNCTION_CALL`'s own children and its own `ARG_LIST`'s direct children
  (never `descendants()`), returning one static reason string per modifier (`DISTINCT`,
  `FILTER`, `WITHIN GROUP`, argument-list `ORDER BY`, `IGNORE`/`RESPECT NULLS`, a named `=>`
  argument, `*`). Wired into `unsupported_emissions` as an `Emission::Template(_)` arm gated on
  `SyntaxKind::FUNCTION_CALL`.
- 10 in-module unit tests in `emission_check.rs` (one per modifier plus the nested-call trap and
  the `OVER`-is-not-a-modifier negative control) — all parse real SQL via `smelt_parser::parse`.
- Two new `emission_ownership` gates: `every_rewrite_id_states_why_it_is_not_a_template` (parses
  `RewriteId`'s doc comments out of `signatures.rs`, per-variant, requiring a `Not a template: …`
  line) and `the_template_interpreter_holds_no_target_text` (asserts `print_template` and
  `is_compound_argument`'s bodies hold no double-quoted string literal).
- `Not a template: …` doc lines added to `RewriteId::BigQueryMedian` (output shape differs by
  position) and `RewriteId::WithinGroupToAnalytic` (reads the `WITHIN GROUP` clause, which a
  placeholder cannot address).
- `template:X` bullet added to the coverage table's cell-vocabulary legend
  (`crates/smelt-db/tests/dialect_audit/report.rs`); `docs/reference/dialect-coverage.md`
  regenerated via `SMELT_REGEN_DOCS=1`.

## Decisions

- No spec delta needed — `multi_backend.md` §"Template emission" already stated the refusal
  rule and the per-`RewriteId` justification line normatively (plan 03's call).
- Confirmed empirically (not from the plan's guessed names) that `WITHIN_GROUP_CLAUSE` and
  `FILTER_CLAUSE` are direct children of `FUNCTION_CALL` (siblings of `ARG_LIST`), while
  `DISTINCT_KW`, `NULL_TREATMENT_CLAUSE`, `ORDER_BY_CLAUSE`, `NAMED_PARAM` are direct children of
  `ARG_LIST` — matching the plan's guess.
- A bare `*` argument is *not* a single-layer `EXPRESSION(STAR)` as the plan implied — it's
  `EXPRESSION(EXPRESSION(STAR))` (one wrapper per `parse_expression` recursive-descent layer).
  `is_star_expression` peels `EXPRESSION` wrappers of arbitrary depth rather than checking one
  level, discovered by dumping the real parse tree for `COUNT(*)`.

## For the next planner

- Phase 4 registers the first function-call template row; its plan already anticipates adding
  the end-to-end compile-path modifier-refusal test there (moved from this phase per the
  2026-09-06 decision-log entry — no production template is call-shaped yet, so there was no
  live call site to refuse against in phase 3).
- Editing `signatures.rs` doc comments shifts every line-number key in
  `.claude/unknown-census.toml` that falls after the edit point — this phase's edit shifted 5
  keys by +10 lines; all five were updated here. Future phases touching `signatures.rs` should
  expect and check for the same drift (flagged in phase 2's summary too).
- Nothing else surfaced outside this phase's task list.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-dialect --lib emission_check` — 10 passed
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test modulo_lowering --test power_lowering --test snapshots` — all passed
- `cargo test -p smelt-db --test dialect_audit` — 51 passed (includes doc-sync gate)
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance` — 15 passed
- `git diff .claude/hardening-baseline.txt` — empty
- `cargo test -p smelt-types --test unknown_census` — 4 passed (after updating 5 shifted line-number keys)
