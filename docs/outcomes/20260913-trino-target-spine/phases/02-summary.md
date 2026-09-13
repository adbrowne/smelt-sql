# Phase 2 summary — `DialectId::Trino` + `SqlDialect::Trino` land exhaustively

## Shipped

- `DialectId::Trino` added to `smelt-types/src/dialect_id.rs`: `ALL` now has 4 entries,
  slug `"trino"`, round-trips.
- `SqlDialect::Trino` in `smelt-dialect/src/dialect.rs`: `name() == "Trino"`,
  `id() == DialectId::Trino`, both language-property predicates (`supports_aggregate_filter_clause`,
  `supports_interval_range_frame`) land conservative `false`, doc comments name phase 8 as the
  measuring phase.
- `type_conformance::type_cast_sql` gained a named `(dt, SqlDialect::Trino)` arm (was absorbed by
  `_ =>`); pinned by `trino_cast_spelling_is_a_named_arm`.
- `smelt-state` ledger/observed-delta/tombstone: Trino joins Spark's refusal arms everywhere
  (`UnsupportedLedgerDialect`), doc comments extended to name Iceberg's shared per-table-commit
  atomicity with Delta (2026-09-13 ruling).
- `smelt-logical::maintenance::availability::state_structure::realisable_state_structures`:
  Trino joins Spark's empty-`vec![]` arm.
- `smelt-backend::maintenance_dialect` is now `Result<MaintenanceDialect, UnsupportedMaintenanceDialect>`
  (new typed error mirroring `UnsupportedLedgerDialect`); no `MaintenanceDialect::Trino` variant
  added (that's `20260913-trino-incremental`'s ~150-arm job). ~8 call sites threaded with `?`/`.map`.
- `smelt-runtime::schema_evolution::ddl_backend_for_dialect` is now
  `Result<DdlBackend, UnsupportedDdlDialect>`; Trino refused by name rather than aliased onto
  `DdlBackend::Spark`'s Delta DDL. Callers in `smelt-cli` tests updated (`.unwrap()`).
- `smelt-runtime::compile.rs`'s hand-restated `dialect_name` match replaced with
  `self.dialect.id().slug()` — single ownership; new test
  `as_struct_dialect_name_comes_from_the_slug` covers all 4 dialects end-to-end through
  `SqlCompiler::compile`.
- Wildcard audit: `crates/smelt-db/tests/dialect_audit/{fixture.rs,probe.rs}` got named
  `unreachable!()` Trino arms (fixture/probe generation for Trino is `20260913-trino-emission`'s
  job); `crates/smelt-db/tests/prop_helpers/generators.rs::join_key_cast` converted its `_ =>`
  fallback to a named 3-arm match including Trino, doc comment flags the spelling as an unverified
  guess (no Trino oracle exists yet).
- `smelt-db/tests/dialect_audit/main.rs`: added `AUDITED_DIALECTS` (DuckDb/Spark/BigQuery only,
  excludes Trino) — see Decisions below.

## Decisions

- **`AUDITED_DIALECTS` (new, not in the plan) scopes `dialect_audit`'s own iteration separately
  from `DialectId::ALL`.** The plan's wildcard-audit task correctly converted `fixture.rs`'s and
  `probe.rs`'s per-dialect matches to named `unreachable!()` arms for Trino. But `dialect_audit/
  main.rs` has 4 tests that loop over `DialectId::ALL` directly (fixture-column coverage, probe
  printing, and the gap-count baseline), and those loops now hit the new `unreachable!()` arms for
  Trino, since Trino is a member of `ALL`. Cross-engine emission audit coverage for Trino is
  explicitly out of scope for this outcome (owned by `20260913-trino-emission`). Rather than build
  a Trino fixture/probe/baseline entry now (out of scope) or leave `DialectId::ALL` non-exhaustive
  (violates criterion 2), added a test-local `AUDITED_DIALECTS` const (3 dialects) that
  `dialect_audit/main.rs`'s 4 `DialectId::ALL`-driven tests iterate over instead. `DialectId::ALL`
  itself is untouched and still exhaustive.
- Large-file baseline bumped for 5 files (`compile.rs` +87, `execute/project/mod.rs` +15,
  `dialect_audit/main.rs` +13, `schema_evolution.rs`(cli tests) +2, `generators.rs` +6) — all from
  threading `Result` through ~20 call sites plus new tests. Reviewed each diff; growth is
  proportionate to the required refactor, not scope creep.

## For the next planner

- Phase 3 (target-config surface) can proceed; `DialectId::Trino`/`SqlDialect::Trino` are fully
  wired with no residual wildcard absorbing them (confirmed by `rg -n '_ =>'` audit — the one
  remaining implicit default is `Signature::engine_native`'s `Native` fallback, already a named
  Known Divergence owned by `20260913-trino-emission`).
- `20260913-trino-emission` inherits: `dialect_audit`'s fixture/probe/baseline work for Trino
  (currently refused via `unreachable!()`), and measuring the two conservative-`false` language
  properties.
- `20260913-trino-ledger` inherits: the Trino ledger refusal arms landed here should flip to real
  implementations once ledger-on-Iceberg is designed.
- `prop_helpers/generators.rs::join_key_cast`'s Trino spelling (`CAST(1 AS INTEGER)`) is an
  unverified guess, flagged in its doc comment — worth confirming once a Trino oracle exists.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy both feature sets, shellcheck, full
  workspace test, example_diagnostics) — all green after two fix rounds (fmt diffs from `Result`
  threading; large-file ratchet baseline bump; `dialect_audit` Trino-in-`ALL` fix).
- `cargo test -p smelt-types --lib dialect_id` — 3 passed
- `cargo test -p smelt-dialect` — 106 passed across all binaries (incl. `capability_conformance`,
  `emission_ownership`, `template_emission`, `operand_conditional`)
- `cargo test -p smelt-state --test ledger_dialect` — 18 passed
- `cargo test -p smelt-cli --test trino_spec_freshness` — 4 passed (phase 1's gate stays green)
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance --test statement_parity` — 65 passed
- `cargo test -p smelt-core --test hardening_budget` — PASS, no baseline drift (`git diff .claude/hardening-baseline.txt` empty)
- `cargo test -p smelt-db --test dialect_audit` — 61 passed (was 4 failing before the `AUDITED_DIALECTS` fix)
