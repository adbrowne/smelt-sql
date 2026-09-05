# Phase 2 summary — `Emission::Template`, build-time validation, generic interpreter

## Shipped

- `Emission::Template(&'static str)` in `crates/smelt-types/src/signatures.rs`, plus
  `TemplateError`, `validate_template`, and `is_call_shaped_template` (all pure, all
  re-exported from `smelt_types`).
- `RewriteId::ModuloCall`/`RewriteId::PowerCall` deleted; `%` now carries
  `Emission::Template("MOD({0}, {1})")` on BigQuery, `^`/`**` carry
  `Emission::Template("POWER({0}, {1})")` on Spark and BigQuery.
- The registry seed's `insert` closure calls `validate_template` for every
  `Emission::Template` row and panics on a malformed one — a build-time gate,
  not a runtime one.
- `print_template` in `crates/smelt-dialect/src/printer.rs` (`pub`, re-exported):
  one generic interpreter dispatched from both `emit_registered_function`
  (positional args from `FunctionCall::arguments()`) and `emit_registered_operator`
  (`BinaryExpr::left()`/`right()`). `print_modulo_call`/`print_power_call` deleted.
- `docs/reference/dialect-coverage.md` regenerated (`template:…` cells replace
  `rewrite:ModuloCall`/`rewrite:PowerCall`); `docs/specs/multi_backend.md`'s
  "No template verdict exists yet" divergence bullet deleted.
- New `crates/smelt-dialect/tests/template_emission.rs` (tests 1, 9, 10 from the
  plan) plus tests 3–8 in `smelt-types/tests/registry_coverage.rs`.

## Decisions

- **Argument-level wrapping is gated on the whole template being call-shaped,
  not applied unconditionally per compound argument.** A call-shaped template
  (`MOD({0}, {1})`) never wraps any argument — comma-separated call arguments
  are already unambiguously delimited, and this is what the pinned
  byte-identity tests (`modulo_lowering.rs`, `power_lowering.rs`) require. A
  non-call template (`{0} - {1}`, none registered yet) wraps a compound
  argument at the substitution site *and* wraps its own whole output. This
  reconciles the plan's general "compound argument is parenthesised" language
  with the requirement that today's two migrations stay byte-identical.
- **"Atom vs compound" is classified by node kind after peeling exactly the
  transparent `EXPRESSION` wrapper the parser puts around every function
  argument and every parenthesised group.** There is no `PAREN_EXPR` `SyntaxKind`
  in this grammar (confirmed by reading `syntax_kind.rs` and the parser) — the
  plan's design-decision text used that name informally. Empirically verified
  (via a throwaway debug test, since removed) that `FunctionCall::arguments()`
  returns the `EXPRESSION` node whose span is exactly the *inner* content for a
  parenthesised argument (e.g. `f((a + b))`'s argument prints as `a + b`, no
  literal parens) — so there is no "double-wrap an already-parenthesised
  argument" case to special-case; a parenthesised and an unparenthesised
  compound argument behave identically once past this accessor.
- **`Emission::Template` gets its own arm in `docs/reference/dialect-coverage.md`'s
  cell renderer** (`report.rs::emission_label`: `template:{t}`) rather than
  reusing `rewrite:{id:?}` — the plan's phase-3 note that "no coverage-table
  schema change lands here" is read as "no new *column*", not "no new cell
  text"; the migrated rows must still render (and did, once regenerated).

## For the next planner

- Phase 3 (`emission_ownership` extended for templates; modifier refusal) can
  build directly on `is_call_shaped_template`/`is_compound_argument` — both are
  already correct for the corpus this phase covers (no non-call template is
  registered yet, so phase 5/7's operand-conditional and Spark-gap work is the
  first real exercise of the non-call path beyond the synthetic tests here).
- No new gaps surfaced outside this phase's task list. One process note: the
  `.claude/unknown-census.toml` allowlist is line-number-keyed, and ~200 new
  lines in `signatures.rs` shifted five existing entries — caught only by the
  `unknown_census` gate failing with "STALE ALLOWLIST"/"UNREGISTERED" pairs at
  old/new line numbers. Worth remembering for any future phase that adds
  substantial code above existing `Unknown`-construction sites in a file
  already on the census.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature
  sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-types --test registry_coverage` — 90 passed.
- `cargo test -p smelt-dialect --test template_emission --test emission_ownership --test modulo_lowering --test power_lowering --test snapshots --test capability_conformance` — all passed.
- `cargo test -p smelt-db --test dialect_audit` — 51 passed (doc-sync gate
  regenerated once, green after).
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity` — all passed.
- `cargo test -p smelt-db --test integration registry_consistency` — 6 passed.
- `git diff .claude/hardening-baseline.txt` — empty (no new production
  `unwrap`/`expect`).
