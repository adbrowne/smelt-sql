# Phase 4 plan — close the DuckDB emission gaps (#177)

## Objective

Give every DuckDB row the audit currently records as an *emission* gap a real verdict —
`Emission::Template` where DuckDB spells the built-in as a shape over the same arguments,
`Emission::Unsupported { reason }` where DuckDB has no such function at all — delete the
corresponding `ledger.rs` rows, and tighten `dialect_gaps_duckdb`. Advances criterion 5
(DuckDB ≤ 5, every survivor a `type_gap`) and completes criterion 3's end-to-end leg, which
phase 3 could not ship because no production template was call-shaped.

## Spec delta

None. `docs/specs/multi_backend.md` §"Template emission" and §"Operator lowering" already
state both mechanisms normatively; this phase only adds registry rows measured against the
live engine. `docs/reference/dialect-coverage.md` is regenerated (data, not spec).

## The seven rows, and the verdict each gets

Measured against DuckDB (`select …` on the pinned library, 2026-09-06) — never from docs:

| Row | Verdict |
|---|---|
| `PERCENTILE_CONT` / `PERCENTILE_DISC` at `Position::Window` | **already** `Emission::Unsupported` in `signatures.rs`; the two ledger rows are redundant — delete them |
| `INITCAP`, `TO_CHAR`, `QUOTE_IDENT`, `QUOTE_LITERAL` | `Emission::Unsupported { reason }` — DuckDB 1.5.x has no such scalar (`Catalog Error: … does not exist`), and none has a placeholder-expressible equivalent (`TO_CHAR`'s format string is not `strftime`'s) |
| `DATE_SUB` | `Emission::Template("{0} - {1}")` — DuckDB spells interval subtraction infix; its own `date_sub(VARCHAR, ts, ts)` is a different function |

`DATE_SUB` is the first **function-call** template row, so it is also the subject of the
end-to-end modifier refusal.

### Contingency (measure, do not guess)

Templating `DATE_SUB` makes its probe *run*, so the type leg fires on it for the first time.
DuckDB returns `TIMESTAMP` for `date - INTERVAL`; smelt's own `binary.rs` already agrees
(`DATE ± INTERVAL → Timestamp`), but the registry declares `DATE_SUB → Date` and the
`DATE_ADD` ledger row records inference yielding `Unknown(Dynamic)` for the unquoted
`INTERVAL 1 DAY` argument. If the type leg mismatches:

1. First try the minimal correction: infer the unquoted `INTERVAL <n> <unit>` literal as
   `DataType::Interval`, and set `DATE_ADD`/`DATE_SUB`'s registry return type to
   `Timestamp { with_timezone: false }` to match `binary.rs` and the engine. This closes
   `DATE_ADD`'s type row too (count 4).
2. Only if that measures wrong, register a `type_gap` for `DATE_SUB` under `#176`, leave the
   count at 6, and say so plainly in the summary — do **not** edit criterion 5.

Boundary: `EXPLODE`, `UNNEST`, `FIRST`, `LAST` stay as they are. No other inference change.

## Tests (red first)

- `dialect_seam::a_template_call_carrying_distinct_is_refused_for_duckdb` — a model with
  `DATE_SUB(DISTINCT d, INTERVAL 1 DAY)` fails `execute`-path compilation with
  `UnsupportedOnBackend` naming `DISTINCT`. The end-to-end leg of criterion 3.
- `dialect_seam::a_model_using_initcap_is_refused_for_duckdb` — `Unsupported` reaches the
  user as a compile-time diagnostic rather than an engine error.
- `template_emission::date_sub_lowers_to_infix_subtraction_on_duckdb` — printed SQL is
  `d - INTERVAL 1 DAY`, and a compound first argument is parenthesised.
- `dialect_audit::gap_count_ratchet` — fails "STALE baseline" until the baseline is tightened.
- `dialect_audit` schema+type legs — the live proof each verdict is right; a wrongly-claimed
  `Unsupported` fails with "the engine accepts it".

## Tasks

1. Add the four `Emission::Unsupported` rows to `INITCAP`, `TO_CHAR`, `QUOTE_IDENT`,
   `QUOTE_LITERAL` at `(DialectId::DuckDb, Position::Any)`, each reason naming what DuckDB
   lacks and what the author should write instead.
2. Add `Emission::Template("{0} - {1}")` to `DATE_SUB` at `(DuckDb, Position::Any)`.
3. Delete the seven closed rows from `crates/smelt-db/tests/dialect_audit/ledger.rs`
   (5 gaps + the 2 redundant `PERCENTILE_*` window rows).
4. Run `cargo test -p smelt-db --test dialect_audit` and read the failures: this is the
   measurement. Apply the contingency above if the `DATE_SUB` type leg mismatches.
5. Tighten `dialect_gaps_duckdb` in `.claude/dialect-gaps-baseline.txt` to the measured count,
   with a dated sign-off comment naming this phase.
6. Write the two `dialect_seam` tests and the `template_emission` print test.
7. Regenerate `docs/reference/dialect-coverage.md` (`SMELT_REGEN_DOCS=1`).
8. Check `.claude/unknown-census.toml` for line-number drift from the `signatures.rs` edits
   (phases 2 and 3 both hit this) and `git diff .claude/hardening-baseline.txt` is empty.
9. Write `phases/04-summary.md`: the measured verdicts, the contingency outcome, the final
   DuckDB count, and anything phase 5 (`OperandClass`) should know.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test dialect_audit` (DuckDB schema+type legs in-process, doc-sync,
  ratchet, ledger two-sidedness)
- `cargo test -p smelt-runtime --test dialect_seam --test projection_dialect_invariance`
- `cargo test -p smelt-dialect --test emission_ownership --test template_emission --test snapshots`
- `cargo test -p smelt-db --test integration registry_consistency`
- `cargo test -p smelt-types --test unknown_census`

## Commit message

`feat(dialect): close the DuckDB emission gaps with templates and Unsupported verdicts`
