# Phase 6e plan — the interval-literal spelling gap

## Objective

No `INTERVAL` spelling currently works end to end on live Trino, so the repair fixture's Form B
band fails obligation 4 (`RepairSliceUnbounded`) before the sidecar question is reached. Make the
ANSI spelling `INTERVAL '<n>' <UNIT>` — the one Trino executes — parse in `smelt-parser` and be
recognised by the shared interval classifier, widening the same classifier to the bare-numeric
`INTERVAL <n> <UNIT>` form the parser already accepts, then re-enable 6c's parked live test.
Advances criterion 2 (the per-group-recompute family executes) and criterion 3 (the named,
explain-visible sidecar-less downgrade is proved live rather than only offline).

## Spec delta (first)

- `docs/specs/model_properties.md`, the paragraph beginning "Every `INTERVAL '<value>'` literal
  this walk encounters is parsed by one shared parser" (~line 127): restate the shared parser's
  accepted surface as the three spellings — `INTERVAL '<n> <unit>'`, `INTERVAL '<n>' <UNIT>`,
  `INTERVAL <n> <UNIT>` — folding identically to `Offset::Seconds`/`Offset::Symbolic`, with the
  unit read from outside the quotes when it is written there. State explicitly that the symbolic
  (month/year) fail-closed classification applies across all three spellings, so no spelling can
  smuggle a calendar-variable offset past the bound derivation.
- `docs-site/docs/reference/language.md`: the interval-literal surface gains the
  `INTERVAL '<n>' <UNIT>` spelling alongside the two already documented. Timeless wording.

## Tests (red first)

Parser (`crates/smelt-parser/src/parser/tests.rs`):
1. `interval_quoted_number_bare_unit_parses` — `WHERE d BETWEEN TIMESTAMP '2025-01-14' - INTERVAL
   '3' DAY AND TIMESTAMP '2025-01-14'` parses with zero errors (today: `Expected AND_KW, found IDENT`).
2. `interval_quoted_number_bare_unit_roundtrips` — the parsed CST prints back byte-identically.
3. `interval_quoted_string_alias_is_not_absorbed` — `SELECT INTERVAL '3' foo` does **not** swallow
   `foo` as a unit (only the recognised unit keywords are absorbed); guards the new lookahead.

Classifier (`crates/smelt-logical/src/analysis/source_bounds.rs` `mod tests`):
4. `interval_quoted_number_bare_unit_folds_to_days` — `INTERVAL '3' DAY` → `Seconds::days(3)`
   (today: 3 *seconds* — a silently wrong offset, not merely unrecognised).
5. `interval_bare_number_bare_unit_folds_to_days` — `INTERVAL 3 DAY` → `Seconds::days(3)`
   (today: `None`, or a misgrab of an unrelated later quoted literal in the same text).
6. `interval_bare_unit_month_is_symbolic` — both new spellings with `MONTH`/`YEAR` classify as
   `Offset::Symbolic`, and `has_symbolic_interval_in_bound_position` returns `true` for each.
7. `form_b_band_admits_quoted_number_bare_unit` — a `BETWEEN <anchor> - INTERVAL '3' DAY AND
   <anchor>` band derives the same `Bounded` reach as the `'3 days'` spelling.

Monotonicity (`crates/smelt-logical/src/analysis/monotonicity.rs` tests or
`crates/smelt-logical/tests/`):
8. `event_time_shift_by_quoted_number_bare_unit` — `ts + INTERVAL '1' HOUR` traces with
   `Offset::Seconds(3600)`, not `Seconds(1)`.

Live Trino (`crates/smelt-cli/tests/trino_incremental_families/degraded_routes.rs`):
9. `per_group_recompute_matches_full_refresh_on_trino` — 6c's parked function becomes a real
   `#[test]` (still skipping on unset `SMELT_TRINO_URL`), with the fixture's band rewritten to
   `INTERVAL '3' DAY`; the second run after the in-place source mutation is row-identical to the
   `--full-refresh` oracle.

## Tasks

1. Land the spec + user-doc delta above.
2. `smelt-parser`: in the typed-literal branch of `parse_primary` (`parser/expr.rs` ~line 694),
   when the type keyword is `INTERVAL` and the token after the string literal is an IDENT naming a
   recognised unit (`YEAR|MONTH|WEEK|DAY|HOUR|MINUTE|SECOND` ± plural), absorb it into the same
   `EXPRESSION` node. Share the unit-keyword predicate with `is_numeric_interval`'s branch.
3. `smelt-logical`: give `source_bounds` one owner for interval-literal recognition that handles
   all three spellings — scan forward from the `INTERVAL` keyword, take either a quoted value or a
   bare number, then an optional trailing bare unit keyword, and reject (fail closed) rather than
   grabbing a later unrelated quoted literal. Route `parse_quoted_interval_offset`,
   `parse_quoted_interval`, `parse_interval_seconds_before`, `extract_interval_from_{addition,
   subtraction}` and `has_symbolic_interval_in_bound_position` through it. Keep the existing
   `parse_interval(value)` unit folding as the single unit table.
4. `smelt-logical`: make `monotonicity::parse_interval_literal` consume the same owner (token-level
   input) so `INTERVAL '1' HOUR` folds to 3600s instead of 1s, and `INTERVAL 1 HOUR` folds at all.
5. Rewrite the repair fixture's band in `degraded_routes.rs` to `INTERVAL '3' DAY`, drop the
   `#[allow(dead_code)]`, restore `#[test]`, and shorten the parked doc comment to a one-line
   history note.
6. Run the live leg against `scripts/trino-up.sh`; if the fixture now admits but a *different*
   product gap surfaces, record it and stop — do not absorb a new gap into this phase.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-parser --quiet` and `cargo test -p smelt-parser-compat --test duckdb_differential --quiet`
  (the new spelling must parse **and** print back to SQL DuckDB still executes; no gap-baseline change)
- `cargo test -p smelt-logical --quiet` (walk_coverage included)
- `cargo test -p smelt-db --test type_property_tests --quiet` (interval typing unaffected)
- Live tier: `source scripts/trino-env.sh` then
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1`, with
  `per_group_recompute_matches_full_refresh_on_trino` green. Coordinator unreachable ⇒
  `<<PHASE_BLOCKED>>`, never a green skip.

## Commit message

`feat(logical): recognise the ANSI INTERVAL '<n>' <UNIT> spelling end to end so Trino's Form B repair band admits`
