# Phase 4 summary — the `PIVOT` decision on Trino

## Shipped

- `docs/specs/multi_backend.md`: §"Clause-level dialect refusals" gains a third bullet
  (`UNPIVOT`) and its intro widens from "two" to "some" clauses; §"Operator lowering"
  (~line 376-389) corrected to state the measured split between `PIVOT` and `UNPIVOT` on
  Trino, replacing the "decision this outcome settles" placeholder.
- `crates/smelt-dialect/src/dialect.rs`: new `SqlDialect::supports_unpivot()` (DuckDB/
  SparkSQL/BigQuery → `true`, Trino → `false`), placed beside
  `supports_aggregate_filter_clause`. `BackendCapabilities::supports_pivot`'s doc comment
  narrowed to state it covers `PIVOT` only, pointing to the new method for `UNPIVOT`.
- `crates/smelt-dialect/src/emission_check.rs`: an `UNPIVOT_CLAUSE` check ahead of the
  registry-driven match (it isn't a registry-backed call), gated on
  `!dialect.supports_unpivot()`, producing `UnsupportedEmission { name: "UNPIVOT", .. }`.
- Tests: `trino_clause_lowering.rs` gains `trino_keeps_pivot_native`,
  `trino_refuses_unpivot_clause`, `duckdb_keeps_unpivot_native`; `compile.rs` gains
  `trino_compile_refuses_unpivot` and `trino_compile_keeps_pivot_native`.
- `.claude/large-file-baseline.txt`: `compile.rs` entry bumped 4233→4306 lines (the two new
  compile-path tests) — sign-off note below.

## Decisions

- **The live probe split the plan's assumption.** `PIVOT (COUNT(id) FOR cat IN ('a'))`
  executes cleanly against a live Trino coordinator (`trino --execute`, inside
  `smelt-trino-coordinator`); `UNPIVOT (val FOR name IN (a,b,c))` fails to parse:
  `mismatched input 'UNPIVOT'`. The plan (written before this measurement) assumed both
  measured `false` together, under one `supports_pivot` flag. They don't — Trino is the
  first dialect where `PIVOT` and `UNPIVOT` diverge. **Ruling:** `BackendCapabilities::
  supports_pivot` for Trino stays `true` (it already was — no code change, no census/matrix
  flip); a *new*, separate dialect-layer fact `SqlDialect::supports_unpivot()` gates the
  compile-time refusal, which fires only for `UNPIVOT`. `PIVOT` gets no lowering and no
  refusal at the dialect layer — it needs neither, since it's Native.
- **Why no lowering for either, even though `PIVOT` measures Native.** The universal
  diagnostic-layer refusal (`DiagnosticCode::UnsupportedConstruct`,
  `check_unsupported_constructs`) already blocks *both* `PIVOT` and `UNPIVOT` for every
  backend, including DuckDB and Spark, which have always executed `PIVOT` natively — the
  reason is output-column derivation, not grammar support. So this phase's dialect-layer
  refusal is purely the `compile_with_sql` backstop the plan described, scoped now to just
  `UNPIVOT` since that's the only one Trino's grammar can't even parse.
- **`.claude/trino-emission-census.txt` unchanged (232 rows, confirmed via `git status` and
  `wc -l`)** — `PIVOT`/`UNPIVOT` are clauses, not `BuiltinRegistry` entries, so this phase
  moves no census row, as the plan anticipated.
- **Sign-off: `large-file-check.sh --update` ran** for `crates/smelt-runtime/src/compile.rs`
  (4233→4306 lines). The growth is the two new compile-path tests the plan's own test list
  requires; `SqlCompiler`/its test helpers are `pub(crate)` (Run Pipeline Parity invariant),
  so these tests can only live inline in `compile.rs`'s existing `#[cfg(test)] mod tests`,
  not a separate integration-test file.

## For the next planner

- Phase 5/6 (audit legs) can now regenerate `dialect-coverage.md` including this corrected
  `PIVOT`/`UNPIVOT` split; no follow-up work is deferred from this phase.
- Nothing else surfaced needing a new row — phase 3's flagged large-file drift (six
  unrelated files) was already resynced in phase 3 and is not reopened here.

## Gates

- `cargo test -p smelt-dialect --test trino_clause_lowering --test capability_conformance --test emission_ownership` — PASS (7 + 11 + 7)
- `cargo test -p smelt-runtime --lib compile::tests` — PASS (43, incl. the 2 new)
- `cargo test -p smelt-db --test dialect_audit` — PASS (69, census leg unmoved at 232 rows)
- `cargo test -p smelt-cli --test trino_emission_spec_freshness` — PASS (7)
- `cargo test -p smelt-backend-trino --test capability_probes probe_supports_pivot` (live, `scripts/trino-up.sh` + `source scripts/trino-env.sh`) — PASS
- `cargo test -p smelt-core --test large_file_ratchet` — PASS (after `--update`)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, shellcheck, workspace tests, example_diagnostics)
