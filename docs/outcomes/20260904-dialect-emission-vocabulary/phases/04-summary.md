# Phase 4 summary — close the DuckDB emission gaps (#177)

## Shipped

- `crates/smelt-types/src/signatures.rs`: `INITCAP`, `TO_CHAR`, `QUOTE_IDENT`, `QUOTE_LITERAL`
  each carry `Emission::Unsupported { .. }` at `(DuckDb, Position::Any)` — measured live against
  the pinned DuckDB 1.5.4 library (not the system `duckdb` CLI, which is a stale 1.4.4 and
  genuinely lacks `initcap`; 1.5.4 does not, confirmed by the audit's own two-sided check).
  `DATE_SUB` carries `Emission::Template("{0} - {1}")` at `(DuckDb, Position::Any)` — the first
  function-call template row.
- `crates/smelt-db/tests/dialect_audit/ledger.rs`: deleted the two `PERCENTILE_CONT`/`PERCENTILE_DISC`
  Window `gap_at` rows (redundant with the registry's own `Unsupported`) and the five
  `INITCAP`/`TO_CHAR`/`QUOTE_IDENT`/`QUOTE_LITERAL`/`DATE_SUB` `gap` rows. Added two new
  `type_gap` rows for `DATE_ADD`/`DATE_SUB` (see Decisions).
- `.claude/dialect-gaps-baseline.txt`: `dialect_gaps_duckdb` 12 → 6, sign-off comment dated.
- `docs/reference/dialect-coverage.md` regenerated.
- New tests: `smelt-runtime/tests/dialect_seam.rs::a_template_call_carrying_distinct_is_refused_for_duckdb`
  (criterion 3's end-to-end leg, deferred from phase 3) and `::a_model_using_initcap_is_refused_for_duckdb`;
  `smelt-dialect/tests/template_emission.rs::date_sub_lowers_to_infix_subtraction_on_duckdb`.
- `.claude/unknown-census.toml`: one line-number entry shifted by the `signatures.rs` edits.

## Decisions

- **DuckDB CLI vs. pinned library**: the ambient `duckdb` CLI at `~/.local/bin` is v1.4.4; the repo
  pins `libduckdb.so` v1.5.4 via `mise run setup-duckdb`. Measuring against the wrong one nearly
  produced a false `Unsupported` claim for `INITCAP` — DuckDB 1.5.4 has it natively. Caught by the
  audit's own `a_ledger_row_the_engine_now_accepts_is_reported_stale`-style check (a declared
  `Unsupported` the engine actually accepts fails loudly), not by manual measurement. Always verify
  against the linked library, never the ambient CLI.
- **`a_ledger_row_the_engine_now_accepts_is_reported_stale` rewritten**: it used to borrow a live
  DuckDB Schema-leg `gap()` row (`"INITCAP"`) as a stand-in probe name. Phase 4 closes every such
  row, so nothing was left to borrow — and mixing dialects (print for Spark, run on the DuckDB
  oracle) to fake one produces a genuine Spark-only-syntax parse error, not the intended scenario.
  Extracted the decision `probe_schema_once` delegates to into a pure `classify_accepted(bool, bool)
  -> AcceptedVerdict`, unit-tested directly with three cases (stale, unsupported-but-accepted —
  including the priority order when both are true — and plain accepted). No dependency on which
  ledger rows happen to be live.
- **Contingency step 2 taken, not step 1**: the plan's guess was that the unquoted `INTERVAL 1 DAY`
  literal infers `Unknown`. Measured directly — it already infers `Interval` correctly on its own.
  The real cause is one level up: `DATE_ADD`/`DATE_SUB` have no `SqlFunction` enum variant in
  `smelt-types/src/functions.rs`, so `infer_function_type` returns `None` before
  `try_registry_inference` ever runs — the registry's return type (corrected here to
  `Timestamp{with_timezone:false}`, matching `binary.rs` and the live engine, since it's the
  semantically right value even though currently unreachable) is never consulted. This is a
  registry-wiring gap, not an unquoted-literal one. Landed the type-correctness fix (harmless,
  forward-looking) but registered fresh `type_gap` rows for both names under #176 per the
  contingency's bail-out clause, landing the count at 6 instead of 4.

## For the next planner

- **New discovery, not in #176's original scope as written**: `DATE_ADD`/`DATE_SUB` are entirely
  unrecognised by `infer_function_type`'s dispatch (no `SqlFunction` variant), a strictly worse gap
  than "infers Unknown for one argument shape" — worth a line in #176 or a follow-up issue. Wiring
  them in touches `smelt-types/src/functions.rs`'s `SqlFunction` enum and every exhaustive match over
  it; out of scope for this phase's "close the DuckDB gaps" mandate.
- Phase 5 (`OperandClass`) can proceed as planned; nothing here reshapes it.
- `.claude/dialect-gaps-baseline.txt` sign-off format followed the existing phase-1 entry's style.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-db --test dialect_audit` — 53 passed (DuckDB schema+type legs, ratchet,
  doc-sync, ledger two-sidedness).
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance` — 17 passed.
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test snapshots` —
  passed (snapshots unaffected).
- `cargo test -p smelt-db --test integration registry_consistency` — 6 passed.
- `cargo test -p smelt-types --test unknown_census` — 4 passed (after the line-number fix).
