# Phase 4 plan — the `PIVOT` decision on Trino

## Objective

Settle criterion 3: `PIVOT`/`UNPIVOT` on a Trino target is answered by an explicit,
measured decision rather than by the capability matrix's inherited `✓`. The ruling
(recorded in the outcome's decision log with this plan) is **refuse, do not lower** —
so this phase makes `supports_pivot` honest for Trino, gives the flag a live consumer
in the compile-path refusal, and proves the refusal with fixtures. Also advances
criterion 8 (refusal happens on the compile path) and criterion 12.

**The ruling and why.** smelt already refuses `PIVOT`/`UNPIVOT` for *every* target at
the diagnostic layer (`check_unsupported_constructs`, `DiagnosticCode::UnsupportedConstruct`:
output columns depend on data values and cannot be determined at compile time). A Trino
lowering would have to enumerate the `IN`-list values and name the resulting columns —
precisely the projection smelt declines to derive, and one it must not recover from
printed SQL (`architecture.md` §"Source-derived projection"). There is therefore no
lowering to admit, and the only gap is that the *dialect* layer would happily print
`PIVOT` verbatim to Trino if a model ever reached it (`compile_with_sql` runs no
diagnostics query). This phase closes that gap with a clause-level refusal keyed on the
flag, in the same place §"Clause-level dialect refusals" already puts the aggregate
`FILTER` and `INTERVAL`-frame refusals — no printer branch on `SqlDialect::Trino`.

**Measure before writing.** The spec currently contradicts itself: §Surface's matrix row
and the prose at `multi_backend.md:91` say Trino accepts `PIVOT`, while §"Operator
lowering" (line 383) says it was measured `false`. The first task settles this against a
live coordinator, as phase 3 settled `//`. If the tier cannot be brought up, emit
`<<PHASE_BLOCKED>>` — do not guess the flag. If the probe measures **true** (Trino does
accept `PIVOT`), that is a live surprise, not a failure: state the `Native` verdict,
correct line 383 instead of line 54, drop the refusal tasks, and record it in the
decision log.

## Spec delta (made first, by the implement step)

`docs/specs/multi_backend.md`:
- §Surface capability matrix, `supports_pivot` row — Trino cell `✓` → `✗`.
- §Surface, the Spark-prior paragraph (~line 91) — move `PIVOT` out of the "both accept"
  list into the broken-flag list (seven → eight flags), stating Trino's grammar has no
  `PIVOT` clause.
- §"Clause-level dialect refusals" — a third bullet: `PIVOT`/`UNPIVOT`, declared as
  `SqlDialect::supports_pivot`, refused at compile time with `UnsupportedOnBackend`
  naming the construct and the backend; never emitted verbatim. State that the
  diagnostic-layer refusal (`UnsupportedConstruct`, all dialects) is the primary gate
  and this is the compile-path backstop for entry points that run no diagnostics query.
- §"Operator lowering" (~line 383) — replace the "whether `PIVOT` gets a lowering …
  is a decision this outcome settles" sentence with the settled ruling and its reason.

## Tests (red first)

- `smelt-dialect/tests/trino_clause_lowering.rs::trino_refuses_pivot_clause` — a parsed
  model containing `PIVOT (…)` yields one `UnsupportedEmission` from
  `unsupported_emissions(.., SqlDialect::Trino, ..)`, naming `PIVOT`.
- `…::trino_refuses_unpivot_clause` — same for `UNPIVOT`.
- `…::duckdb_keeps_pivot_native` — the same tree on DuckDB yields no refusal (the flag
  is consulted, not hardwired).
- `smelt-dialect/tests/capability_conformance.rs` — `cell!(trino, supports_pivot, false,
  "Trino")`, plus the existing doc-sync leg re-read against the corrected matrix row.
- `smelt-runtime` `compile.rs` unit test `trino_compile_refuses_pivot` (beside
  `trino_compile_refuses_materialized_view_without_native_ivm`) — compiling a `PIVOT`
  model for a `trino` target errors with a message carrying `UnsupportedOnBackend`,
  `PIVOT` and the backend name; no SQL is produced.
- `…::duckdb_compile_keeps_pivot` — the same model compiles for a `duckdb` target, so
  the refusal is dialect-scoped rather than a blanket compile-path block.
- Existing `smelt-db/src/tests.rs::test_pivot_rejected_with_diagnostic` /
  `test_unpivot_rejected_with_diagnostic` stay green — the primary gate is unchanged.

## Tasks

1. `bash scripts/trino-up.sh` + `source scripts/trino-env.sh`; run the `PIVOT` probe
   statement (`SELECT * FROM (VALUES (1,'a'),(2,'a')) AS t(id,cat) PIVOT (COUNT(id) FOR
   cat IN ('a'))`) and record the verbatim engine response in the phase summary. Tier
   unreachable → `<<PHASE_BLOCKED>>`.
2. Make the four spec edits above.
3. `BackendCapabilities::trino_iceberg().supports_pivot` → `false`, with the measured
   error text as the inline comment (the house style in that constructor).
4. Add `SqlDialect::supports_pivot()` next to `supports_aggregate_filter_clause` —
   `DuckDB | SparkSQL | BigQuery => true`, `Trino => false` — with the doc comment
   explaining it is a language property.
5. Wire the clause-level refusal into `smelt-dialect/src/emission_check.rs`: a
   `PIVOT_CLAUSE`/`UNPIVOT_CLAUSE` arm producing `UnsupportedEmission` with a
   `PIVOT_UNSUPPORTED` reason constant, checked the same way the `FILTER`-clause arm is.
   `UnsupportedEmission.name` is `&'static str` — use `"PIVOT"`/`"UNPIVOT"`.
6. Write the tests above; red before green.
7. Update `capability_conformance.rs` cell and run `capability_probes.rs::
   probe_supports_pivot` live — it asserts measured == flag, so it must now pass rather
   than skip.
8. Confirm `.claude/trino-emission-census.txt` is **unchanged** (232 rows): `PIVOT` is
   not a registry entry, so this phase moves no census row. State that in the summary.
9. Write `phases/04-summary.md` (shipped / decisions / for the next planner / gates).

## Verification

- `cargo test -p smelt-dialect --test trino_clause_lowering --test capability_conformance --test emission_ownership`
  — emission_ownership must stay green with no `SqlDialect::Trino` branch in the printer.
- `cargo test -p smelt-runtime --lib compile::tests`
- `cargo test -p smelt-db --test dialect_audit` (census leg unmoved)
- `cargo test -p smelt-cli --test trino_emission_spec_freshness`
- `cargo test -p smelt-backend-trino --test capability_probes probe_supports_pivot` (live)
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(dialect): refuse PIVOT/UNPIVOT on Trino at compile time and correct the measured supports_pivot flag`
