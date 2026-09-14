# Phase 6 summary — the live Trino value leg, two-sided ledger, census retired at zero

## Shipped

- `TrinoClient::execute_json` (`crates/smelt-backend-trino/src/client.rs`) — the `/v1/statement`
  paging loop factored into a shared `follow_pages` helper, reused by `execute`, `execute_schema`
  and the new `execute_json`, which returns undecoded `serde_json::Value` cells alongside the
  reported `(name, raw_type)` columns.
- `cell_from_trino_json` + `impl ValueOracle for TrinoOracle` (`crates/smelt-oracle-testkit/src/
  trino_oracle.rs`) — decodes Trino's raw JSON cells (including NaN/Infinity spelled as strings,
  decimals as strings, and nested `array(...)`) against their own declared type, bypassing
  `arrow_convert` entirely.
- `value_leg_trino` + `trino_caret_agrees_with_duckdb_power` (`crates/smelt-db/tests/dialect_audit/
  trino.rs`) — both legs now live for Trino; 125 probes compared.
- 8 new Trino value-leg ledger rows (`ledger.rs`): 7 `Divergent` (permanent semantic differences —
  `CORR`/`REGR_SLOPE` NULL-vs-NaN on degenerate variance, `GREATEST`/`LEAST` NULL propagation,
  `DATE_TRUNC` millisecond-precision rendering, `JSON_ARRAY_LENGTH` strict-array-only semantics,
  `SPLIT_PART` NULL-vs-empty-string on out-of-range index) and 1 `Gap` (`JSON_ARRAY`'s missing
  `NULL ON NULL` lowering, #209). `dialect_gaps_trino` 57 → 58.
- `census.rs` rewritten: `BOTH_LEGS_LIVE` now includes Trino; the file-backed shrink-only census
  (`read_census`/`write_census`/`diff`/`CENSUS_HEADER`, `SMELT_REGEN_TRINO_CENSUS`, and the four
  file-bound tests) is deleted along with `.claude/trino-emission-census.txt` and its `.gitignore`
  whitelist line. `classify` now takes the both-legs set as an explicit parameter; the standing
  gate is `no_dialect_has_unverified_pairs` over `DialectId::ALL`/`AUDITED_DIALECTS`.
- Spec (`docs/specs/multi_backend.md`): the census paragraph in §"Cross-engine emission audit" is
  now generic (no longer names the Trino file); the "No `dialect_audit` Trino value leg" Known
  Divergences entry is deleted.
- `report.rs`'s Trino verification-tier row: "schema only" → "schema + value"; `docs/reference/
  dialect-coverage.md` regenerated.

## Decisions

- NaN/Infinity decode: Trino's JSON protocol has no numeric spelling for these (JSON has none), so
  they arrive as strings `"NaN"`/`"Infinity"`/`"-Infinity"`. `f64::from_str` parses all three
  case-insensitively — no custom parsing needed.
- Followed Spark's existing convention of unscoped (`position: None`) `divergent()` rows rather
  than introducing position-scoped divergence tracking, even though `CORR`/`REGR_SLOPE` only
  diverge at `Position::Window` (their `Aggregate`/`WholePartitionWindow` forms agree). Matches
  the precedent already set for Spark's identical `CORR`/`REGR_SLOPE` rows rather than adding new
  machinery for one outcome.
- `JSON_ARRAY`'s NULL-omission is registered as a `Gap` (closable, tracked under #209) rather than
  `Divergent`: a variadic `NULL ON NULL` template could close it, unlike the other seven, which are
  genuine, permanent engine-semantics differences.

## For the next planner

- Phase 7 (`Restructure`/`Rewrite` on Trino) and phase 8 (seams) are next per the outcome's table;
  nothing here was deferred out of phase 6's own scope.
- `JSON_ARRAY`'s `NULL ON NULL` lowering is a real, closable gap under #209 — worth a look whenever
  #209 gets picked up in bulk, since it's the one Trino value-leg row that isn't permanent.
- The full-workspace-`cargo test`-with-Trino-tier-exported concurrency collision on the shared
  Iceberg REST catalog (phase 5's finding) still applies — this phase again ran targeted live
  commands, then `verify-phase.sh` with the tier unexported.

## Gates

- Live (tier exported): `cargo test -p smelt-backend-trino --quiet` (17 passed), `cargo test -p
  smelt-oracle-testkit --quiet` (74+3 passed), `cargo test -p smelt-db --test dialect_audit
  --quiet` (70 passed, including `value_leg_trino`, `schema_leg_trino`,
  `trino_caret_agrees_with_duckdb_power`).
- Offline (tier unexported): `cargo test -p smelt-types --test registry_coverage` (109 passed);
  `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test
  operand_conditional` (11 passed); `cargo test -p smelt-cli --test trino_emission_spec_freshness`
  (7 passed); `cargo test -p smelt-db --test dialect_audit --quiet` (70 passed).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck,
  workspace `cargo test`, `example_diagnostics`).
- `git ls-files .claude/` shows no `trino-emission-census.txt`; `git status --short` shows only the
  intended file changes plus the deletion.
