# Phase 5 summary — the live `dialect_audit` Trino schema leg

## Shipped

- `TrinoClient::execute_schema` (`crates/smelt-backend-trino/src/client.rs`): submits, follows
  `nextUri` to completion, returns reported `(name, type)` pairs, decodes no rows.
- `TrinoOracle` (`crates/smelt-oracle-testkit/src/trino_oracle.rs`): `TypeOracle` impl over a held
  current-thread runtime, `from_env()` gated on `SMELT_TRINO_URL`; a narrow `row_count(sql)` helper
  (not a full `ValueOracle`) used only to prove the fixture executes.
- `crates/smelt-db/tests/dialect_audit/trino.rs`: `schema_leg_trino`, live, gated.
- Trino joined `AUDITED_DIALECTS`; `probe::print_for` and `fixture.rs` (types, `ARRAY[...]`
  literal, the `INTERVAL '<n>' DAY` day-time literal workaround) gained Trino arms, each spelling
  measured live rather than assumed.
- `census.rs`: `Coverage::Verified` now consults a new `BOTH_LEGS_LIVE` set, separate from
  `AUDITED_DIALECTS` — Trino is in the latter (schema leg) but not the former (no value leg yet),
  so no census row silently flips to `Verified` on the schema leg alone.
- 57 `ledger.rs` Gap rows for Trino (53 unregistered-builtin, 4 type-leg mismatches), tracked in
  bulk under new issue [#209](https://github.com/adbrowne/smelt-sql/issues/209); `error_class.rs`
  gained Trino's `BackendError` refusal-message prefixes.
- `report.rs`/`coverage_table.rs`: Trino column and verification-tier row; regenerated
  `docs/reference/dialect-coverage.md`.
- `.github/workflows/compat.yml`: a third `trino-integration` step running `dialect_audit`,
  path filter widened to the touched crates.
- `docs/specs/multi_backend.md` §Known Divergences narrowed to "no value leg yet".

## Decisions

- **`LOG`'s one-argument form got a real registry fix, not a ledger row.** Live probing showed
  Trino's `log(x)` has no one-argument overload (only `log(x, base)`, which succeeds with implicit
  BIGINT→DOUBLE widening). A single ledger row can't express "gap on this arm, pass on that arm"
  when one of the two failing probes has `arm: None` (a row with no arm exempts every arm,
  including the one that now passes) — so `numeric.rs` gained a genuine
  `Emission::Conditional` verdict for `DialectId::Trino` (arity 1 → `Unsupported`, otherwise →
  `Native`), mirroring Spark's existing shape for the same entry. This generalizes correctly
  rather than papering over one probe.
- **`REPEAT` is two different functions under one name.** Trino's `repeat(element, count)`
  builds an array; DuckDB's `repeat(string, count)` repeats a string. Recorded as a `gap` with
  that explanation rather than a type mismatch, since no rename or cast closes it.
- **`verify-phase.sh` must run with `SMELT_TRINO_URL` unset.** Running the full workspace
  `cargo test` with the Trino tier exported triggers namespace/transaction collisions between
  concurrently-scheduled test binaries hitting the single shared Iceberg REST catalog (verified:
  `smelt-backend-trino`'s own suite is 100% clean run alone, twice, but collides under full-suite
  concurrency). This reproduces the pattern phases 3/4 already followed (targeted live commands
  run separately, then `verify-phase.sh` clean with the tier unexported) — not a regression this
  phase introduced.

## For the next planner

- The full-workspace-`cargo test`-with-Trino-tier-up concurrency collision (`Namespace already
  exists`, `Cannot determine whether the commit was successful`) is real and worth a tracking
  issue if any future workflow wants to run the whole suite live against Trino at once — today
  every phase avoids it by running targeted live commands first. Not raised as a new issue here
  since it doesn't block this outcome's own gates.
- Phase 6 (value leg) will need `BOTH_LEGS_LIVE` extended to include Trino, a `ValueOracle` impl
  on `TrinoOracle`, and the ledger/ratchet/census widened accordingly — the schema-leg
  infrastructure this phase built (fixture spellings, `AUDITED_DIALECTS` membership, the 57 gap
  rows) is what it builds on.

## Gates

- `cargo test -p smelt-backend-trino --test statement_client` — PASS (8)
- `cargo test -p smelt-oracle-testkit` (live) — PASS (75 incl. doctests)
- `cargo test -p smelt-db --test dialect_audit` (live) — PASS (72; `COVERAGE[trino schema]`
  printed; census leg at 217 rows, `Trino` schema-only classification confirmed by
  `a_schema_only_dialect_is_not_yet_verified`)
- `cargo test -p smelt-types -p smelt-dialect -p smelt-backend-trino -p smelt-oracle-testkit` —
  PASS
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test
  operand_conditional --test capability_conformance` — PASS
- `cargo test -p smelt-db --test integration registry_consistency` — PASS
- `cargo test -p smelt-runtime --test dialect_seam` — PASS
- `bash .claude/scripts/shellcheck-gate.sh` — PASS
- `bash .claude/scripts/verify-phase.sh` (SMELT_TRINO_URL unset) — ALL GREEN
