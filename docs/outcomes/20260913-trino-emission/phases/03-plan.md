# Phase 3 plan — explicit Trino verdicts for the operator and clause divergences

## Objective

Give every infix-operator registry entry an explicit `DialectId::Trino` verdict, and pair the
four clause-shaped divergences (`QUALIFY`, `::`, trailing commas, `[a, b]`) with printed-output
assertions proving the already-measured `BackendCapabilities` flags actually drive Trino's
lowering. Advances criteria 2 and 7, and shrinks `.claude/trino-emission-census.txt` from 237
rows to 232 (criterion 1's ratchet, first turn of the crank).

## Spec delta (comes first)

`docs/specs/multi_backend.md` §"Operator lowering", the `//` paragraph (currently lines ~318–327).
It claims Trino's integral arm "lowers to the same `DIV(a, b)` template as GoogleSQL and Spark
SQL". **Trino has no `DIV` function**, and Trino's `/` over two integers already truncates toward
zero — the same semantics DuckDB's `//` has for integral operands — while `/` over floating or
decimal operands is plain division, which is also what DuckDB's `//` degrades to. So on Trino the
whole operand axis collapses: `//` is a single unconditional `Template("{0} / {1}")`, with no
per-class arms and no unresolved-arm refusal, because there is no class for which the spelling
differs. Rewrite that sentence to say exactly this and to state *why* Trino differs from the other
two dialects here (no `DIV` function; `/` is already class-sensitive in the same direction as
DuckDB's `//`).

**Settle it live before writing it.** Bring the tier up (`bash scripts/trino-up.sh`, then
`source scripts/trino-env.sh`) and execute `SELECT 7/2`, `SELECT -7/2`, `SELECT 7.5/2.0`,
`SELECT DIV(7,2)` against the coordinator; record the four answers in the phase summary. If the
tier cannot be brought up, do **not** block — this phase's deliverable is offline registry data,
and phase 6's value leg is the gate that must never skip green. In that case write the verdict as
planned and record in the summary that the four answers are unverified, so phase 6 checks them.
If the live answers contradict the analysis above, the live answers win and the spec sentence
records what they were.

## Tests (red first)

1. `smelt-types/tests/registry_coverage/emission.rs::every_infix_operator_states_a_trino_verdict`
   — `stated_emission_at(DialectId::Trino, Position::Scalar)` is `Some(_)` for each of `%`, `^`,
   `**`, `//`, `||`; asserts *stated*, not merely `emission_at`'s default.
2. `smelt-dialect/tests/power_lowering.rs::trino_lowers_caret_and_double_star_to_power` — printing
   `val ^ 2` and `val ** 2` with `SqlDialect::Trino` / `BackendCapabilities::trino_iceberg()`
   yields `POWER(val, 2)`; no `^` and no `**` survives.
3. `smelt-dialect/tests/modulo_lowering.rs::trino_keeps_infix_modulo_native` — `a % b` prints
   verbatim on Trino (Trino has an infix `%`), and no `MOD(` appears.
4. `smelt-dialect/tests/power_lowering.rs::trino_lowers_floor_divide_to_plain_division` — `a // b`
   prints `a / b` on Trino; neither `//` nor `DIV(` survives, for integral, floating and
   type-unresolved operands alike (the unconditional-template claim).
5. New `smelt-dialect/tests/trino_clause_lowering.rs`, four tests over
   `BackendCapabilities::trino_iceberg()`:
   `qualify_is_rewritten_to_an_outer_subquery`, `double_colon_cast_is_rewritten_to_cast_call`,
   `trailing_commas_are_dropped`, `array_literal_brackets_are_kept_native` (Trino is the first
   non-DuckDB dialect where `supports_array_literal` is `true`, so this asserts the *absence* of
   the `ARRAY(...)` rewrite).
6. `smelt-db --test dialect_audit census::tests::the_trino_census_matches_the_registry_exactly`
   — goes red on stale rows the moment the verdicts land, green again after regeneration at 232.

## Tasks

1. Bring up the tier and settle the four `//`-related answers; record them.
2. Edit `docs/specs/multi_backend.md` §"Operator lowering" `//` paragraph per the spec delta.
3. Write tests 1–5 red.
4. In `crates/smelt-types/src/signatures/builtins/infix_operators.rs`, add
   `(DialectId::Trino, Position::Any, …)` rows: `Template("POWER({0}, {1})")` for `^` and `**`;
   `Native` for `%` and `||`; `Template("{0} / {1}")` for `//`. Each carries a doc comment naming
   the measured reason (or, absent a live tier, the reason plus "unverified — phase 6 value leg").
5. Confirm registry-construction validation accepts the new rows unchanged (placeholder range,
   argument coverage, no variadic template, window-position call shape) — no new validation code
   should be needed; if a check rejects a row, that is a finding for the summary, not a lowered
   check.
6. Verify no printer change was required (tests 2–5 must pass on existing generic dispatch). Any
   printer edit that name-matches a Trino spelling or branches on `SqlDialect::Trino` is forbidden
   — `emission_ownership` is the gate.
7. Regenerate the census:
   `SMELT_REGEN_TRINO_CENSUS=1 cargo test -p smelt-db --test dialect_audit census::tests::the_trino_census_matches_the_registry_exactly`;
   confirm 232 rows and that the five operator rows are the ones gone.
8. Write `phases/03-summary.md`; flip the phase-3 row to `done`; append a dated decision-log line
   recording the `DIV` spec correction and the live answers (or their absence).

## Verification

- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-dialect --test power_lowering --test modulo_lowering --test trino_clause_lowering`
- `cargo test -p smelt-dialect --test emission_ownership --test capability_conformance`
- `cargo test -p smelt-db --test dialect_audit`
- `cargo test -p smelt-cli --test trino_emission_spec_freshness`
- `git diff --stat .claude/` — only `trino-emission-census.txt`, shrinking; no baseline lowered
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(dialect): state explicit Trino emission verdicts for the infix operators and pair the clause divergences with printed-output gates`
