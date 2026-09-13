# Phase 5 plan — the live `dialect_audit` Trino leg, schema direction

## Objective

Give the cross-engine emission audit a real Trino schema leg: registry-derived probes printed for
`SqlDialect::Trino`, executed against the live coordinator, with coverage totality enforced over a
fourth dialect and `report.rs` rendering a Trino column. Advances criteria 4 (schema direction and
"an entry with no probe is *named*, never dropped") and 6 (the published table gains its Trino
column). Verification of the implicit-`Native` claims — the census reaching zero — is phase 6's;
this phase deliberately keeps Trino `Unverified` in the census until the value leg exists.

## Spec delta

None. Phase 1 already wrote every sentence this phase implements: §"Cross-engine emission audit"
(Trino's schema leg is a real execution, not a dry run), its gates-by-tier row, and §"CI tiering"'s
never-skip-green rule. The only doc touched here is the generated `docs/reference/dialect-coverage.md`
(regenerated, not authored) and `multi_backend.md` §Known Divergences' "No `dialect_audit` Trino leg"
bullet, which narrows from "no fixture, probe, ledger row or baseline metric, and no Trino column"
to "no value leg yet" — phase 10 deletes it.

## Design decisions this phase settles

- **`Verified` becomes leg-aware.** `census::classify` today returns `Verified` for any
  `AUDITED_DIALECTS` member. Adding Trino to that list would flip all 232 census rows to `Verified`
  on the strength of a schema leg alone — precisely the "unverified ≠ passing" claim this outcome
  exists to make. So `AUDITED_DIALECTS` gains Trino (it is what drives the offline totality gates,
  the fixture gate and the print-for-every-dialect gate), and a *separate* notion — the set of
  dialects with **both** legs live — is what `census::classify` consults for `Verified`. Trino joins
  the first, not the second, until phase 6.
- **Types come from the coordinator's reported column metadata, not from decoded rows.**
  `multi_backend.md` §Known Divergences records that a Trino `array(...)` cell does not decode to
  Arrow yet. The schema leg needs only types, so the oracle reads `columns[].type` and never
  materialises row data — the array gap cannot masquerade as a rejected probe. A result carrying no
  column metadata is an `Err` naming the query, never an empty column list that would read as a pass.
- **Ledger rows.** Phase 6 owns the Trino ledger rows and the ratchet. This phase may add
  `Leg::Schema` rows only for pairs the live coordinator actually rejects, each with a tracking
  issue; `.claude/dialect-gaps-baseline.txt` gains the `dialect_gaps_trino` metric the
  `baseline_names_exactly_the_audited_dialects` gate forces the moment Trino is audited.

## Tests

- `smelt-oracle-testkit`: `trino_oracle_reports_column_types` — live, gated on `SMELT_TRINO_URL`:
  `SELECT CAST(1 AS BIGINT) AS a, CAST('x' AS VARCHAR) AS b` maps to the smelt types via the
  existing `trino_type_to_arrow` + `arrow_to_smelt` pair.
- `smelt-oracle-testkit`: `trino_oracle_errors_on_a_rejected_query` — live: a syntactically invalid
  query returns `Err`, so the leg can distinguish rejection from an empty schema.
- `smelt-backend-trino`: `execute_schema_returns_columns_without_decoding_rows` — against the
  existing `axum` stub, an `array(bigint)` column's metadata comes back while no cell decode runs.
- `dialect_audit::registry_totality::the_fixture_has_a_column_for_every_type_constraint_family` and
  `every_probe_prints_for_every_dialect` — extended by construction (Trino joins `AUDITED_DIALECTS`);
  they must pass with no `unreachable!` hit in `fixture.rs`/`probe.rs`.
- `dialect_audit::registry_totality::the_trino_fixture_executes_and_yields_eight_rows` — live,
  mirroring the DuckDB test, selecting the named columns rather than `*` so the array decode gap is
  not in scope.
- `dialect_audit::trino::schema_leg_trino` — live: `run_schema_leg(DialectId::Trino, &oracle)`, zero
  failures, `probes_compared >= PROBE_COVERAGE_FLOOR`, `COVERAGE[trino schema]` printed.
- `dialect_audit::census`: `unverified_pairs_exist_only_for_unaudited_dialects` becomes
  `unverified_pairs_exist_only_for_dialects_without_both_legs`, and a new
  `a_schema_only_dialect_is_not_yet_verified` asserts Trino classifies `Unverified` while in
  `AUDITED_DIALECTS` — the regression guard for the flip described above.
- `dialect_audit::coverage_table::every_entry_and_dialect_appears_in_the_table` — the label list
  gains `Trino`, and the verification-tier section names Trino's tier.

## Tasks

1. Add `TrinoClient::execute_schema(&self, sql) -> Result<Vec<(String, String)>, BackendError>`:
   submit, follow `nextUri` to completion so errors surface, return the reported `(name, type)`
   pairs, decode no rows. Test against the existing stub.
2. Add `TrinoOracle` to `smelt-oracle-testkit` (`trino_oracle.rs`, exported from `lib.rs`):
   `from_env()` reading `SMELT_TRINO_URL`/`_USER`/`_CATALOG`/`_SCHEMA`, a held current-thread tokio
   runtime, `impl TypeOracle` mapping via `trino_type_to_arrow` + `arrow_to_smelt`. No `ValueOracle`
   impl — that is phase 6's.
3. Teach `fixture.rs` Trino: `ty()` (`VARCHAR`/`INTEGER`/`BIGINT`/`DOUBLE`/`DECIMAL(10,2)`/`BOOLEAN`/
   `DATE`/`TIMESTAMP`/`ARRAY(BIGINT)`/`VARBINARY`/`INTERVAL DAY TO SECOND`), `array_lit` (`ARRAY[…]`),
   and the `iv_interval` literal arm. Measure each spelling against the live coordinator before
   committing it — phase 3's and phase 4's precedent, both of which corrected an assumption.
4. Replace both `DialectId::Trino => unreachable!` arms in `probe::print_for` with `SqlDialect::Trino`
   / `BackendCapabilities::trino_iceberg()`.
5. Add `DialectId::Trino` to `AUDITED_DIALECTS`; add the `dialect_gaps_trino 0` metric to
   `.claude/dialect-gaps-baseline.txt`.
6. Split the census's `Verified` test off `AUDITED_DIALECTS` onto a both-legs-live set, per the
   decision above; confirm `.claude/trino-emission-census.txt` still holds exactly 232 rows.
7. New `crates/smelt-db/tests/dialect_audit/trino.rs` (module registered in `main.rs`) carrying
   `schema_leg_trino`, gated on `SMELT_TRINO_URL`, skipping green locally with the exact string
   `skipping schema_leg_trino`.
8. `report.rs`: Trino column in the entry table, Trino row in the verification-tier table;
   `coverage_table.rs` label list gains `Trino`. Regenerate with
   `SMELT_REGEN_DOCS=1 cargo test -p smelt-db --test dialect_audit the_coverage_table_matches_the_registry`.
9. `.github/workflows/compat.yml` `trino-integration`: add a step running
   `cargo test -p smelt-db --test dialect_audit` with the tier up, `tee`-ing its log and erroring on
   `grep -q "skipping schema_leg_trino"` (targeted, not a bare `skipping` — the same binary's Spark
   and BigQuery legs legitimately skip there).
10. Narrow the §Known Divergences "No `dialect_audit` Trino leg" bullet to the value leg only.
11. Record any live-measured surprise as a `## Decisions` entry in `phases/05-summary.md`, and any
    pair the coordinator rejects as a `Leg::Schema` ledger row with a tracking issue.

## Verification

- `bash scripts/trino-up.sh && source scripts/trino-env.sh` — if the tier cannot be brought up, this
  phase emits `<<PHASE_BLOCKED>>` rather than landing an unexercised leg. The legs are the never-skip-green
  ones; there is no offline fallback here as there was for phase 3.
- `cargo test -p smelt-backend-trino --test statement_client`
- `cargo test -p smelt-oracle-testkit` (live)
- `cargo test -p smelt-db --test dialect_audit` (live; expect `COVERAGE[trino schema]` in the output
  and the census leg unmoved at 232 rows)
- `bash .claude/scripts/verify-phase.sh`
- `bash .claude/scripts/shellcheck-gate.sh` if the workflow step grows a shell body

## Commit message

`feat(dialect-audit): add the live Trino schema leg, fixture and coverage column`
