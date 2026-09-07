# Phase 3 plan — modifier refusal, interpreter ownership gates, `template` in the coverage legend

## Objective

Make a template verdict apply only to a plain positional call: a call carrying a modifier a
placeholder cannot reproduce is refused on the compile path with `UnsupportedOnBackend` naming
that modifier (criterion 3). Extend `emission_ownership` so the interpreter is structurally
barred from holding target text and every `RewriteId` states why a placeholder could not name
its shape (criteria 4-partial, 6). Give the coverage table's legend a `template` entry
(criterion 7).

## Spec delta

None. `docs/specs/multi_backend.md` §"Template emission" already states the refusal rule
("A template applies to a plain positional call, and refuses everything else") and §Constraints
already states "Template interpretation is generic" including the per-`RewriteId` justification
line. This phase implements text that is already normative.

## Tests

Unit tests in `crates/smelt-dialect/src/emission_check.rs` (`#[cfg(test)] mod tests`, parsing
real SQL with `smelt_parser`) over the new pure detector:

1. `distinct_argument_is_refused` — `COUNT(DISTINCT x)` yields a reason naming `DISTINCT`.
2. `filter_clause_is_refused` — `COUNT(x) FILTER (WHERE y > 0)` names `FILTER`.
3. `within_group_is_refused` — `PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x)` names `WITHIN GROUP`.
4. `argument_list_order_by_is_refused` — `STRING_AGG(x, ',' ORDER BY x)` names the argument-list `ORDER BY`.
5. `null_treatment_is_refused` — `LAST_VALUE(x IGNORE NULLS)` names `IGNORE NULLS`/`RESPECT NULLS`.
6. `named_argument_is_refused` — a `f(a => 1)` call names the named argument.
7. `star_argument_is_refused` — `COUNT(*)` names the `*` argument.
8. `a_plain_positional_call_is_admitted` — `MOD(a, b + 1)` yields `None` (negative control).
9. `a_modifier_on_a_nested_call_does_not_refuse_the_outer_call` — `MOD(COUNT(DISTINCT x), 2)`:
   the *outer* call yields `None`. Guards the `descendants()` trap (`FunctionCall::named_params`
   uses `descendants`, so the detector must read the call's own arg list only).
10. `an_over_clause_is_not_a_modifier` — `SUM(x) OVER (PARTITION BY g)` yields `None`.

In `crates/smelt-dialect/tests/emission_ownership.rs`:

11. `every_rewrite_id_states_why_it_is_not_a_template` — each variant parsed out of
    `enum RewriteId` must carry a doc line beginning `Not a template:`; a variant without one
    fails, naming it.
12. `the_template_interpreter_holds_no_target_text` — the source bodies of `print_template` and
    `is_compound_argument` contain no double-quoted string literal (they are true of both today).
    Every character of target-dialect text must arrive from the registry's template string.

## Tasks

1. Empirically confirm the CST shape of each of the seven modifiers: parse each snippet above and
   dump the `FUNCTION_CALL` node's own children/kinds. Do not code against the guessed kind names
   (`DISTINCT_KW`, `FILTER_CLAUSE`, `WITHIN_GROUP_CLAUSE`, `ORDER_BY_CLAUSE` inside `ARG_LIST`,
   `NULL_TREATMENT_CLAUSE`, `NAMED_PARAM`, `STAR` in `ARG_LIST`) until the dump confirms them.
2. Add a pure `fn template_unsupported_modifier(node: &SyntaxNode) -> Option<&'static str>` to
   `emission_check.rs`: inspects only the call node's own children and its own `ARG_LIST`'s direct
   children, returns the first modifier found as a static, user-facing reason string. Private to
   the crate; tested in-module so no public API widens.
3. Reason strings are `&'static str` (one per modifier) so `UnsupportedEmission::reason` stays
   static — phrased for a user, naming the modifier and saying the target spells this built-in as
   a fixed template over its positional arguments.
4. Wire an `Emission::Template(_)` arm into `unsupported_emissions`, reached only for
   `SyntaxKind::FUNCTION_CALL` (an operator `BINARY_EXPR` can carry none of these). No registry row
   is added or changed in this phase — the first function-call template lands in phase 4.
5. Add a `Not a template: …` line to the doc comment of both `RewriteId::BigQueryMedian` (position-
   dependent output shape; the aggregate arm is a `CASE` over an `ARRAY_AGG`, not a substitution)
   and `RewriteId::WithinGroupToAnalytic` (reads the `WITHIN GROUP` sort key and its direction out
   of a clause a placeholder cannot address).
6. Add the two `emission_ownership` gates (tests 11-12).
7. Add a `template:X` bullet to the coverage table's cell-vocabulary legend in the generator
   (`crates/smelt-db/tests/dialect_audit/report.rs`), then regenerate
   `docs/reference/dialect-coverage.md` with
   `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit the_coverage_table_matches_the_registry`.
8. Run the verification gates below.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-dialect --lib emission_check`
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test modulo_lowering --test power_lowering --test snapshots`
- `cargo test -p smelt-db --test dialect_audit` (includes the doc-sync gate)
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance`
- `git diff .claude/hardening-baseline.txt` — expected empty.
- Watch for the line-number-keyed `.claude/unknown-census.toml` drift noted in the phase 2
  summary if `signatures.rs` line counts shift.

## Commit message

`feat(dialect): refuse template calls carrying modifiers a placeholder cannot express`
