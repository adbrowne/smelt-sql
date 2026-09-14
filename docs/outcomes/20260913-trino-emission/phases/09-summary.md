# Phase 9 summary — the Trino type-oracle leg

**Shipped:**
- `TrinoOracle` wired into `type_property_tests.rs` as a fourth live oracle (`TRINO`
  `LazyLock`, `TRINO_COLUMNS_COMPARED`, `TRINO_COLUMN_COVERAGE_FLOOR=50`,
  `check_trino_coverage_floor`, sweep arm, post-sweep floor check, and the matching Trino
  arm on all 6 deterministic smoke-test sites) — same shape as the BigQuery leg.
- `divergences.rs`: `TypeDivergence.trino_type: Option<DataType>` on all 27 existing entries,
  `find_divergence`'s `"trino"` arm, plus new entries found by the live sweep:
  `decimal_arithmetic_model` (widened), `regr_family_trino_real`, `sign_double/integer/
  bigint/decimal` (widened — Trino's SIGN preserves argument type except for DOUBLE),
  `avg_decimal` (widened), `integer_division_trino_truncates`,
  `bigint_division_trino_truncates`, `repeat_trino_returns_array`.
- `crates/smelt-backend-trino/src/arrow_convert.rs`: `trino_type_to_arrow` gained `time`,
  `array(...)`, `row(...)` (with a top-level-comma/space parser handling nesting),
  `INTERVAL ... TO ...`, `json`, and `timestamp(n)/time(n) with time zone` — all found live by
  the phase 9 and (transitively) `dialect_audit` sweeps. 8 new unit tests.
- `trino_oracle.rs`: `query_types` no longer lets `trino_type_to_arrow`'s
  `BackendError::execution_failed("trino", ...)` `Display` pass through unchanged (that shape
  is exactly `TRINO_REFUSALS`'s allow-list) — factored into `map_column_type`, which emits a
  distinct "trino oracle cannot map declared column type: …" message that falls through to
  `Fatal`. `error_class.rs` documents and asserts the new message is not in the allow-list.
  Also: `cell_display`'s `Cell::Null` now renders as empty (matching DuckDB/Spark's own
  array-of-NULL text), not the literal word `"NULL"`.
- `dialect_audit/ledger.rs`: removed 4 now-stale Schema-leg gaps (`ARRAY_AGG`,
  `CURRENT_TIMESTAMP`, `JSON_EXTRACT`, `NOW` — all "type signature the client does not yet
  decode", closed by the arrow_convert fixes) and reclassified `REPEAT` from a Schema gap to
  a `repeat_trino_returns_array` type divergence + a `Leg::Value` `divergent()` row (Trino's
  `REPEAT` builds an array, DuckDB's repeats a string — a real semantic difference, not a
  decode gap). `.claude/dialect-gaps-baseline.txt` tightened 55 → 50 with a sign-off note.
  `docs/reference/dialect-coverage.md` regenerated.
- Spec delta: `docs/specs/multi_backend.md` §"Output-schema type conformance" (four-oracle
  paragraph) and §"CI tiering" (the Trino type-property leg's tier + env gate), plus
  §References.
- `.claude/large-file-baseline.txt`: `type_property_tests.rs` 1744→1910,
  `divergences.rs` newly registered at 1170 (crossed the 1000-line floor) — sign-off note.

**Decisions:**
- Took the "land it" branch of criterion 11, not the deferral branch — the transport, the
  refusal classifier and the Arrow map all already existed, so deferring would have been
  silence dressed as a decision (plan's framing, confirmed correct).
- Genuine Trino type divergences found (not just decode gaps): decimal multiplication growth
  (Trino: p1+p2, smelt/Spark: p1+p2+1), `SIGN` preserves argument type on Trino for
  INTEGER/BIGINT/DECIMAL (only DOUBLE matches Spark's always-Double behaviour), integer/bigint
  `/` truncates on Trino (smelt/DuckDB promote to Double), `REGR_*` reports REAL not DOUBLE,
  `AVG(DECIMAL)` preserves precision/scale (like Spark), `REPEAT` builds an array (different
  function under the same name).
- Fixed `trino_type_to_arrow` itself for `array`/`row`/`time`/`interval`/`json`/`with time
  zone`, rather than registering the resulting Fatal failures as known-unusable — these are
  smelt's own mapping gaps, and Fatal was designed to force exactly this fix, not become a
  new place to park unfixed gaps.

**For the next planner:**
- Coverage floor calibrated from live runs: 256-case sweep = 155 columns, repeated
  1000-case soaks = 443–543. Floor set to 50, matching BigQuery's margin.
- `trino_type_to_arrow` still has no VARBINARY/UUID/IPADDRESS arm (documented in
  `trino_oracle.rs`'s module doc and the `unmappable_declared_type_is_not_a_refusal` test) —
  none of the generators currently produce these, so it's untested territory rather than a
  known gap; worth a note if a future generator adds binary/UUID literals.
- `run_count`'s own doc comment ("Callers must avoid selecting a type `trino_type_to_arrow`
  doesn't yet recognise") is now narrower (VARBINARY/UUID/IPADDRESS only) — not re-verified
  against every existing caller, since none of them select those types today.
- Phase 10 (close) should double check no other dialect_audit ledger rows reference `#209`
  for a schema-leg type-decode reason now fixed here — did a full-suite scan for the four
  names removed but did not exhaustively re-read every #209 row for a similar latent case.

**Gates:**
- `cargo test -p smelt-db --test type_property_tests --quiet` — green, tier absent (93 tests).
- Live (`scripts/trino-up.sh` + `scripts/trino-env.sh`): `prop_type_inference` green at 256,
  512 (×2) and 1000 (×3) cases; `cargo test -p smelt-oracle-testkit`,
  `cargo test -p smelt-backend-trino`, `cargo test -p smelt-db --test dialect_audit` (73/73,
  including the ratchet and doc-sync gates) all green.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  shellcheck, full workspace test, example_diagnostics).
- Ratchets: `dialect-gaps-baseline.txt` tightened (not lowered incorrectly — net gap count
  actually dropped, sign-off note added); `registry-migration-baseline.txt`,
  `parser-gaps-baseline.txt`, `hardening-baseline.txt` untouched.
