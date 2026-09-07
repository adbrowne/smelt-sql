# Phase 2 plan — `Emission::Template`, build-time validation, generic interpreter

## Objective

Add `Emission::Template` to `BuiltinRegistry` as validated static data, interpret it with one
generic printer routine that knows no function names, and retire `RewriteId::ModuloCall` /
`RewriteId::PowerCall` by restating them as templates whose printed output is byte-identical to
today's. Advances success criteria 1, 2 and the phase-2 share of 9.

## Spec delta

`docs/specs/multi_backend.md` §"Known Divergences / Open Questions": delete the bullet
"**No template verdict exists yet; the fixed-shape rewrites live as `RewriteId` variants.**" —
`Emission::Template` now exists and `%`/`^`/`**` are templates. Leave the second bullet
(operand-conditional verdicts) intact; leave `docs/reference/dialect-coverage.md`'s `template`
verdict kind and the architecture/CLAUDE.md invariant text to phases 3 and 8 (no coverage-table
schema change lands here — the migrated rows must still render, so regenerate the doc if the
generator's output text changes and keep the doc-sync gate green).

## Design decisions this phase fixes

- `Emission::Template(&'static str)`. Placeholders are `{n}`, zero-based, over the call's
  *positional* arguments; an infix `BINARY_EXPR` supplies exactly two (`{0}` = left, `{1}` = right).
- Validation is a pure function in `signatures.rs`
  (`fn validate_template(template, &Signature, Position) -> Result<(), TemplateError>`) called from
  the registry seed for every `Emission::Template` row, so a malformed template panics at registry
  construction. Use `assert!`/`panic!` with the `TemplateError` message, **not** `.expect(` — the
  hardening ratchet counts `expect` in production (`.claude/hardening-baseline.txt`).
- Rejected at build time: an index ≥ the signature's fixed arity; a fixed parameter no placeholder
  references; unbalanced parentheses; a non-call-shaped template stated at `Window` /
  `WholePartitionWindow`, or at `Position::Any` for an entry whose `kind` is `Agg`/`Window`; a
  template on a variadic signature (a placeholder cannot name a variadic tail — refuse rather than
  guess).
- "Call-shaped" is structural: an identifier followed by a parenthesised group that closes exactly
  at end of string. Decided from shape, never from a function name.
- Substitution parenthesisation is decided from the argument's **CST node kind**, never its text:
  compound (`BINARY_EXPR`, `CASE`, `CAST`, comparison, unary) → wrapped; atom (literal, identifier,
  column ref, `FUNCTION_CALL`, and `PAREN_EXPR`, which already carries its own parens — wrapping it
  again would break byte identity) → not wrapped.
- A template whose outermost form is not a single call is emitted wrapped in parentheses. Both
  current migrations (`MOD(…)`, `POWER(…)`) are call-shaped, so nothing is wrapped and the pins hold.

## Tests

Red-green, in this order. Steps 1–2 are written **before** any migration so they are green on
today's printer and stay green after.

1. `crates/smelt-dialect/tests/template_emission.rs::modulo_and_power_output_is_pinned` — a corpus
   of `%`, `^`, `**` expressions (bare columns, literals, nested calls, parenthesised operands,
   nested `a % b % c`, an operand that is itself a lowered call, one under `OVER`) printed for
   DuckDB, Spark and BigQuery, asserted against literal expected strings captured from the
   pre-migration printer.
2. Existing `modulo_lowering.rs` / `power_lowering.rs` / `snapshots.rs` stay green (their
   `Emission::Rewrite(RewriteId::PowerCall)` assertions become `Emission::Template("POWER({0}, {1})")`).
3. `smelt-types/tests/registry_coverage.rs::template_index_beyond_arity_is_rejected` — validator
   rejects `{2}` on a two-parameter signature.
4. `…::template_dropping_an_argument_is_rejected` — `MOD({0})` on a two-parameter signature fails.
5. `…::template_with_unbalanced_parens_is_rejected`.
6. `…::non_call_template_at_a_window_position_is_rejected` — `{0} - 1` stated at
   `Position::WholePartitionWindow` fails.
7. `…::template_on_a_variadic_signature_is_rejected`.
8. `…::the_full_registry_builds` — forces `REGISTRY` and asserts every `Emission::Template` row
   validates (the build-time gate, exercised as a test).
9. `crates/smelt-dialect/tests/template_emission.rs::compound_argument_is_parenthesised` — a
   synthetic registry-independent case through the interpreter: a `BINARY_EXPR` argument is wrapped,
   a `PAREN_EXPR` argument is not double-wrapped, an identifier is not wrapped.
10. `…::non_call_template_is_wrapped_in_parens` — a template such as `{0} - {1}` composes correctly
    inside a larger expression.
11. `smelt-dialect/tests/emission_ownership.rs::every_rewrite_id_is_dispatched` — still green with
    `ModuloCall`/`PowerCall` gone (the parser of `RewriteId` variants must not go stale).

## Tasks

1. Run the phase-1 gates once to confirm a clean baseline, then write test 1 and capture the pinned
   strings from the current printer output (record them as literals; do not compute them at runtime).
2. Add `Emission::Template(&'static str)` with a doc comment stating the placeholder grammar and the
   two parenthesisation rules, citing §"Template emission".
3. Add `TemplateError` and `validate_template` beside the `Emission` enum; unit-test it (tests 3–7).
4. Call `validate_template` from the registry seed's `insert` closure for every `Emission::Template`
   row; add test 8.
5. Add `fn print_template(template, args: &[SyntaxNode], ctx, out)` to `printer.rs` — generic
   substitution, structural parenthesisation, whole-template wrapping — and dispatch
   `Emission::Template` at both existing emission sites (the `FUNCTION_CALL` match near
   `printer.rs:2589` and the `BINARY_EXPR` operator match near `printer.rs:2628`).
6. Extract positional arguments once: from a `FunctionCall`'s argument list, or from a
   `BinaryExpr`'s `left`/`right`. Preserve the existing trailing-trivia push behaviour.
7. Replace the `%` and `^`/`**` registry rows with `Emission::Template("MOD({0}, {1})")` and
   `Emission::Template("POWER({0}, {1})")`; delete `RewriteId::ModuloCall`, `RewriteId::PowerCall`,
   `print_modulo_call`, `print_power_call`, and their `apply_rewrite` arms.
8. Update `registry_coverage.rs`'s `RewriteId` inventory list and the `PowerCall` assertion; update
   the `power_lowering.rs` module doc where it names the retired variant.
9. Apply the spec delta; regenerate `docs/reference/dialect-coverage.md` if the generator's rendering
   of these rows changes, and keep the doc-sync gate green.
10. Write `phases/02-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-dialect --test template_emission --test emission_ownership --test modulo_lowering --test power_lowering --test snapshots --test capability_conformance`
- `cargo test -p smelt-db --test dialect_audit`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity`
- `cargo test -p smelt-db --test integration registry_consistency`
- `git diff .claude/hardening-baseline.txt` — must be empty (no new production `unwrap`/`expect`).

## Commit message

`feat(dialect): state fixed-shape lowerings as validated registry templates`
