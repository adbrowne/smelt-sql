# Phase 1 — Retire the PostgreSQL emission dialect (#181)

## Objective

Remove `PostgreSql`/`PostgreSQL` from the emission vocabulary — the `DialectId` and `SqlDialect`
variants, `BackendCapabilities::postgresql()`, the three explicit registry rows, the coverage
table's column, and the `dialect_gaps_postgres` baseline entry — so that every later phase's
templates and conditional arms cannot be forced to carry a verdict no engine can verify. Advances
success criterion 8, and clears the way for criteria 1/5/7 (no unverifiable column to fill).
The pg_query grammar anchor in `smelt-parser-compat` and ROADMAP §"PostgreSQL Backend" stay.

## Spec delta (spec-first — the implement step makes these edits before touching code)

- `docs/specs/multi_backend.md` §Surface "Backends": drop `PostgreSQL` from the declared
  `SqlDialect` list (leaving `DuckDB | SparkSQL | BigQuery`).
- `docs/specs/multi_backend.md` §"Output-schema type conformance": the backend-namespace example
  `postgres.sum(...)` becomes a surviving backend (`bigquery.sum(...)`).
- `docs/specs/multi_backend.md` §"Operator lowering" / §Known Divergences: the `//` refusal is
  "Spark and BigQuery", not "Spark, PostgreSQL and BigQuery"; prose that describes PostgreSQL as an
  *emission target* (the MEDIAN/`^`/`VALUES`/null-safe-equality passages) is reworded to describe it
  only as the grammar anchor where that is what is meant, and dropped where it is not.
- `docs/specs/architecture.md` §Known Divergences: delete the **"PostgreSQL emission verdicts are
  unverified"** entry, replacing it with nothing (the divergence is closed by removal); the pg_query
  entries at §"Parse-level semantic anchor" and §Constraints #13 corpus grounding are untouched.
  §"Additional rewrites apply for the Spark and PostgreSQL printers" loses the PostgreSQL half.

## Tests (red-green)

- `smelt-types dialect_id::tests::all_is_exhaustive` — `DialectId::ALL.len() == 3`; the match lists
  three variants. Red before the variant is deleted.
- `smelt-types dialect_id::tests::slug_round_trips_and_matches_the_existing_spelling` — asserts
  `from_slug("postgres") == None`, i.e. the retired slug does not resolve.
- **new** `smelt-types tests/registry_coverage.rs::no_registry_row_names_a_retired_dialect` —
  scans `signatures.rs` source for the spellings `PostgreSql`/`PostgreSQL`; zero hits. The durable
  gate that stops a later template or arm reintroducing an unverifiable column.
- **new** `smelt-db tests/dialect_audit/main.rs::baseline_names_exactly_the_audited_dialects` —
  two-sided: every `dialect_gaps_*` metric in `.claude/dialect-gaps-baseline.txt` corresponds to a
  `DialectId::ALL` slug and every slug has a metric. A stale entry for a retired dialect fails.
- `smelt-db tests/dialect_audit/main.rs::gap_count_ratchet` — unchanged code, now green over three
  dialects with no `dialect_gaps_postgres` lookup.
- `smelt-db tests/dialect_audit/main.rs::the_coverage_table_matches_the_registry` — green after
  regeneration; the table has three dialect columns.
- **new** `smelt-db queries/maintenance.rs` unit test `retired_backend_names_resolve_to_nothing` —
  `backend_dialect_for("postgres")`/`("postgresql")` return `None` and
  `backend_write_capabilities_for("postgres")` returns the conservative default. Fail-loud posture
  for an unrecognised name, matching `Target::backend_type`, which already rejects `type: postgres`.
- `smelt-logical lowering/as_struct.rs` unit tests + `smelt-db tests/integration/as_struct_tests.rs`
  — the `"postgres"` backend string is no longer supported (`as_struct_to_sql(..., "postgres")`
  returns `None`); the surviving backends are asserted unchanged.
- `smelt-dialect tests/emission_ownership.rs::the_printer_branches_on_no_dialect_variant` — the
  forbidden-spelling list drops `SqlDialect::PostgreSQL` and keeps three; still zero hits.
- `smelt-dialect tests/snapshots.rs` — `qualify_rewrite_postgresql`, `explode_to_unnest_postgresql`,
  `every_unchanged_postgresql` deleted **only after** confirming each construct (QUALIFY rewrite,
  `EXPLODE`→`UNNEST`, `EVERY` native-vs-rename) still has a snapshot on a surviving dialect; where
  it does not, port the case to Spark or BigQuery rather than dropping coverage.

## Tasks

1. Make the spec edits above (spec-first), then work outward from `DialectId`.
2. `crates/smelt-types/src/dialect_id.rs`: delete the `PostgreSql` variant, its `ALL` entry, slug
   arm, and update the two unit tests.
3. `crates/smelt-types/src/signatures.rs`: delete the three `DialectId::PostgreSql` emission rows
   (`//` unsupported, `EXPLODE` rename, and the `EVERY`-family row) and the `needs_cast_for`
   assertion; add the `no_registry_row_names_a_retired_dialect` gate to `registry_coverage.rs` and
   retarget the PostgreSql assertions there to a surviving dialect.
4. `crates/smelt-dialect/src/dialect.rs`: delete `SqlDialect::PostgreSQL`, its `name`/`DialectId`
   arms, `BackendCapabilities::postgresql()`, and `test_postgresql_schema_evolution_capabilities`.
5. Fix the resulting match arms: `smelt-backend/src/lib.rs` maintenance-dialect map (and its
   `merge_columns_guard` test), `smelt-runtime/src/{compile.rs,schema_evolution.rs}`,
   `smelt-db/tests/dialect_audit/probe.rs`, `smelt-cli/src/commands/explain.rs` and
   `smelt-ui/src/build.rs` doc comments.
6. `crates/smelt-db/src/queries/maintenance.rs`: drop the `"postgres" | "postgresql"` arms from
   `backend_write_capabilities_for` and `backend_dialect_for`; add the new unit test.
7. `crates/smelt-logical/src/lowering/as_struct.rs`: drop the now-unreachable `"postgres"` branch
   and its supported-backend list entry; update both test sites.
8. Update the `smelt-dialect` test suites that instantiate `postgresql()` capabilities
   (`median_lowering`, `modulo_lowering`, `percentile_analytic_lowering`, `power_lowering`,
   `pipe_native`, `pipe_lowering`, `snapshots`) — delete the PostgreSQL case where a surviving
   dialect already covers the assertion, port it otherwise.
9. `.claude/dialect-gaps-baseline.txt`: delete `dialect_gaps_postgres 0`, and add a dated sign-off
   comment naming #181, the outcome's 2026-09-04 Decision log entry, and that the metric is removed
   because the dialect is retired (not because gaps were absorbed). Add the new baseline-hygiene
   gate to `dialect_audit/main.rs`.
10. Regenerate the coverage doc:
    `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit the_coverage_table_matches_the_registry`,
    and update the doc's prose if it names PostgreSQL.
11. Close #181 with `gh issue close 181 --comment` citing the Decision log rationale and this phase.
    If `gh` is unavailable, record that in the summary and leave it to phase 8 — never claim it closed.
12. Write `phases/01-summary.md`: what was removed, any snapshot coverage ported rather than
    deleted, whether #181 was closed, and anything phases 2–7 must know (notably: dialect columns
    are now three, so template and arm rows never need a PostgreSQL verdict).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test dialect_audit` (DuckDB legs run in-process)
- `cargo test -p smelt-dialect --test emission_ownership --test snapshots --test capability_conformance`
- `cargo test -p smelt-types --test registry_coverage`
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test restructure_multiplicity`
- `cargo test -p smelt-db --test integration registry_consistency`
- `git diff --stat crates/smelt-parser-compat docs/ROADMAP.md` shows the pg_query anchor untouched.

## Commit message

`refactor(dialect): retire the PostgreSQL emission dialect (#181)`
