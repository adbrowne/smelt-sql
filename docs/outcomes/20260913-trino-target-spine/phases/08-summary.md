# Phase 8 summary — the capability profile, established by execution

## Shipped

- `BackendCapabilities::trino_iceberg()` (`crates/smelt-dialect/src/dialect.rs`) — every flag
  measured against a live coordinator, replacing phase 6's provisional all-`false` profile.
- `crates/smelt-backend-trino/tests/capability_probes.rs` — 27 live-gated tests, one probe per
  matrix flag (plus the two spec-only rows and the two `SqlDialect` language properties), each
  asserting the measured verdict against `trino_iceberg()`/`SqlDialect::Trino`; a totality test
  (`every_capability_field_has_a_probe`) parses the spec table directly so a forgotten flag
  fails by name instead of silently passing.
- `SqlDialect::Trino::supports_aggregate_filter_clause()` and
  `::supports_interval_range_frame()` flipped `false` → `true` (both measured true).
- `docs/specs/multi_backend.md` §Surface capability matrix: every Trino `?` replaced with a
  measured value; the "unmeasured" paragraph replaced with the prior-held/prior-broke summary;
  the "Trino capability column is unmeasured" Known Divergences entry deleted.
- `crates/smelt-cli/tests/trino_spec_freshness.rs`: `trino_capability_column_exists_and_is_unmeasured`
  → `trino_capability_column_is_measured` (inverted: no `?` cells, row-count parity with DuckDB);
  new `trino_unmeasured_divergence_is_gone`.
- `crates/smelt-dialect/tests/capability_conformance.rs`: Trino column added to
  `every_flag_matches_matrix`.
- `TrinoBackend::capabilities()` now returns `BackendCapabilities::trino_iceberg()`; the
  `BackendType::Trino` arm in `smelt-maintenance-testkit`'s `print_body_for_dialect` no longer
  `unimplemented!()`s.
- `BackendCapabilities` gained `#[derive(PartialEq)]` so the backend test can assert whole-struct
  equality against the constructor.

## Decisions

- Measured 27 flags live; two (`supports_retraction`, `supports_fingerprint_sidecar`) are
  architectural/implementation facts rather than engine-executable capabilities, so they are
  named explicitly in the totality gate's carve-out list rather than given a trivial always-false
  probe. Full rationale and every quoted error in the outcome's Decision log (2026-09-14 entry).
- `supports_column_scoped_merge` is probed via an explicit partial `SET lbl = s.lbl`, not
  DuckDB/Spark's `SET *` shorthand — the spec's own definition of the flag ("recomputing only
  the group's columns... passing every other column through unchanged") describes the former;
  Trino does reject `SET *` specifically, which is not what this flag gates.
- `supports_array_literal` is probed with `cardinality([1,2,3])`, not a bare `SELECT [1,2,3]` —
  Trino's own array-literal parse/exec succeeds, but our Arrow decode of `array(integer)` result
  values is a separate, pre-existing gap unrelated to this flag.

## For the next planner

- Phase 9 (end-to-end) needs to update `dialect_and_capabilities`'s Trino refusal list: it will
  now see `supports_qualify`, `supports_double_colon_cast`, `supports_trailing_commas`,
  `supports_alter_column_using`, `supports_pipe_syntax`, `supports_pipe_set_drop_rename`,
  `supports_transactional_ddl`, `supports_insert_overwrite`, `supports_native_ivm` and
  `supports_merge_schema_write` as `false` (refusal-worthy), and every other matrix flag as
  `true` — a materially less degenerate profile than the prior all-`false` one.
- Not done here (deliberately out of scope): decoding a Trino `array(...)` result column to
  Arrow. `supports_array_literal`'s probe sidesteps it via `cardinality()`; a real model emitting
  an array-typed projection through `execute_sql`/`get_preview` will still fail. Flagging for
  whichever outcome first needs Trino array-typed output.
- `supports_transactional_ddl = false` is a property of smelt's own stateless HTTP client
  (no session continuity across `START TRANSACTION`/DDL/`ROLLBACK`), not necessarily an
  inherent Trino/Iceberg limit — worth re-measuring if the client ever grows session-token
  forwarding.

## Gates

- `bash scripts/trino-up.sh && source scripts/trino-env.sh && cargo test -p smelt-backend-trino --test capability_probes` — 27 passed, 0 skipped.
- `cargo test -p smelt-backend-trino --lib` — 16 passed.
- `cargo test -p smelt-dialect --test capability_conformance` — 2 passed.
- `cargo test -p smelt-cli --test trino_spec_freshness` — 5 passed.
- `cargo test -p smelt-dialect --test emission_ownership` — 11 passed (no printer-side dialect
  branch introduced).
- `cargo fmt --all -- --check` — clean.
- `bash .claude/scripts/verify-phase.sh` run with `SMELT_TRINO_URL` unset — see commit for
  final confirmation (offline conformance gates green, live legs skip green).
