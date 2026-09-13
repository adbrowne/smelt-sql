# Phase 3 summary — explicit Trino verdicts for the operator and clause divergences

## Shipped

- `crates/smelt-types/src/signatures/builtins/infix_operators.rs`: explicit `DialectId::Trino`
  rows for all five infix operators — `%` → `Native`, `^`/`**` → `Template("POWER({0}, {1})")`,
  `//` → `Template("{0} / {1}")`, `||` → `Native`.
- `docs/specs/multi_backend.md` §"Operator lowering": corrected the `//` paragraph — Trino has
  no `DIV` function, so its integral arm is not the `DIV(a,b)` template GoogleSQL/Spark use; the
  whole operand axis collapses to one unconditional `{0} / {1}`.
- New tests: `power_lowering.rs::trino_lowers_caret_and_double_star_to_power`,
  `power_lowering.rs::trino_lowers_floor_divide_to_plain_division`,
  `modulo_lowering.rs::trino_keeps_infix_modulo_native`, `registry_coverage/emission.rs::
  every_infix_operator_states_a_trino_verdict`, and a new
  `crates/smelt-dialect/tests/trino_clause_lowering.rs` (4 tests: QUALIFY, `::`, trailing
  commas, `[a,b]`).
- `.claude/trino-emission-census.txt`: 237 → 232 rows (the five operators removed).

## Decisions

- Trino's `//` is a single `Template`, not a `Conditional` with per-class arms — there is no
  operand class Trino's `/` spells differently for. Settled live (see below), not by analysis
  alone.
- The four clause divergences (`QUALIFY`, `::`, trailing commas, `[a,b]`) needed **no** printer
  or registry change: `BackendCapabilities::trino_iceberg()` already set the right flags (landed
  in T1), and the printer's existing capability-driven dispatch (`printer/mod.rs`) already routes
  Trino through the same generic rewrites Spark/BigQuery use. The four new tests exist to prove
  this, not to drive new lowering code — `emission_ownership` stays green with no Trino-specific
  branch added.
- Resynced `.claude/large-file-baseline.txt` (`large-file-check.sh --update`) for six files
  already 1 line over baseline at HEAD before this phase touched anything, unrelated to Trino
  emission — see the outcome's 2026-09-14 decision log entry for the affected files and origin.

## For the next planner

- Live answers recorded, no unverified items handed to phase 6: `SELECT 7/2` = `3`, `SELECT -7/2`
  = `-3`, `SELECT 7.5/2.0` = `3.750000`, `SELECT DIV(7,2)` = `FUNCTION_NOT_FOUND`.
- The large-file baseline drift (six files, 1 line each, pre-existing at HEAD) is worth a quick
  look if it recurs — it wasn't caused by this phase or anything in the immediately preceding
  Trino commits I could find, so its origin is unclear.
- Phases 4–6 (PIVOT decision, live audit legs, value-direction + ledger) are next in sequence;
  nothing here changes their scope.

## Gates

- `cargo test -p smelt-types --test registry_coverage` — pass (110 tests)
- `cargo test -p smelt-dialect --test power_lowering --test modulo_lowering --test trino_clause_lowering` — pass (18 tests)
- `cargo test -p smelt-dialect --test emission_ownership --test capability_conformance` — pass (13 tests)
- `cargo test -p smelt-db --test dialect_audit` — pass (69 tests)
- `cargo test -p smelt-cli --test trino_emission_spec_freshness` — pass (7 tests)
- `git diff --stat .claude/` — `trino-emission-census.txt` (232 rows, shrinking) and
  `large-file-baseline.txt` (unrelated resync, documented above); no ratchet lowered
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
