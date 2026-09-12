# Phase 9 — the two known live conformance failures, characterised and gated offline

## Objective

Close success criterion 6: `dags_bigquery::diamond_propagation_suffices_on_bigquery` and
`gate_composed_bigquery::composed_keyed_pool_upholds_equivalence_on_bigquery` are each named
with their mechanism, their fixing commit, and their live evidence — both are already **fixed
and live-confirmed** in the repo record, not open. The phase's product is the durable half:
an offline gate that would have caught the diamond mechanism before any live sweep, the one
missing seam test, and the retirement of the stale "uncharacterised" claims that still sit in
the BigQuery conformance binary. Also serves criterion 3 (every fixed construct gated offline).

**The record this phase writes down** (assembled at plan time; the implement step verifies each
citation rather than re-deriving it):

| Test | Mechanism | Fix | Live evidence |
|---|---|---|---|
| `diamond_propagation_suffices_on_bigquery` | `diamond_dag`'s `ParityFilter` body renders `WHERE id % 2 = 0`; GoogleSQL has no infix `%` (`400 Syntax error: Expected ")" but got "%"`, measured live 2026-08-19). Chasing it found the worse sibling: infix `^` is bitwise XOR on GoogleSQL, so it returns a *different number* rather than erroring. | `7a2eb89d0` (`%`→`MOD`), `af972abe0` (`^`→`POWER`) | targeted re-run 2026-08-19 (231.65s, pass); whole sweep 2026-08-21 (21/21); concurrent sweep 2026-08-22 (22 cases) |
| `composed_keyed_pool_upholds_equivalence_on_bigquery` | No mechanism of its own — collateral from three already-closed gaps it reached in one case: the keyed-fold `MERGE`'s not-matched arm hardcoding `INSERT *` (`build_cumulative_merge_sql` took no dialect), `Backend::execute_model`'s unconditional `DROP VIEW`/`DROP TABLE` across an object-type mismatch, and the composed route-3 delta query's hand-rolled `FROM (VALUES …) AS t(...)` row set. | `0178e6bd4`, `d84320a44`, `e028596e3`/`aee113753` | confirmed live 2026-08-19 sweep (14/21, this case in the passing set); same two sweeps above |

The one thing this phase cannot do is re-confirm green at **today's** HEAD (the live leg needs
credentials the loop does not have, and phases 1-8 have touched maintenance emitters since
2026-08-22). That debt is recorded by name and date — never skipped green.

## Spec delta

`docs/specs/multi_backend.md` §"Known Divergences" — one new entry:
**"The BigQuery conformance leg's live evidence has a date."** State that the leg's last
all-green live sweep is 2026-08-22 (22 cases, 621.61s, 4-way concurrent), that every case
since is verified offline only, and that a re-sweep is owed whenever maintenance emission or
the shared testkit render surface changes. Name the offline gates that stand in between
(`googlesql_render`, `modulo_lowering`, `power_lowering`, `require_merge_columns`,
`no_family_hardcodes_a_backend_dialect`). No other spec section changes — the mechanisms
themselves are already specified (§"Operator lowering", §"Row sets", the keyed-MERGE entry).

## Tests

New file `crates/smelt-maintenance-testkit/tests/googlesql_render.rs`:

1. `every_dag_body_prints_clean_googlesql` — for every `DagBody` variant, parse
   `dag::render_node_body` and print it with the BigQuery dialect; assert the printed text
   carries no refused construct (infix `%`, infix `^`, `MEDIAN(`, `VARCHAR`, `DOUBLE`,
   `EXCEPT ALL`, a `FROM (VALUES` constructor). This is the gate that would have caught
   `diamond_propagation_suffices` offline.
2. `every_composed_pool_body_prints_clean_googlesql` — same check over a deterministic sample
   of `render_composed_model_body` / `render_composed_oracle_sql` from the composed keyed pool.
3. `the_refused_construct_scan_is_not_vacuous` — negative control: hand the same scanner a
   string containing each needle in turn and assert it reports each one (the scan cannot pass
   by never matching anything, the same fail-closed shape phase 8 used).
4. `a_body_that_does_not_parse_fails_loud` — an unparseable body must fail the gate, not be
   silently skipped.

New test in `crates/smelt-cli/tests/maintenance_conformance_bigquery/backend.rs` (compiles only
under `--features smelt-cli/bigquery`; needs no warehouse and no `SMELT_BQ_PROJECT`):

5. `bigquery_oracle_relation_issues_no_ddl_and_returns_an_inline_subquery` — the override
   returns `(<s_select_sql>) AS <oracle_table_name>` and issues no statement against the
   backend it is handed (`CREATE OR REPLACE TEMPORARY VIEW` is refused by GoogleSQL). The
   backend argument is ignored by the override; pass an in-memory DuckDB backend.

## Tasks

1. Verify each citation in the table above against the repo (`git show` the four fix commits;
   confirm `modulo_lowering`/`power_lowering` pass at HEAD) — correct the table if any is wrong.
2. Write test 3 (negative control) and the shared refused-construct scanner; red first.
3. Write tests 1, 2 and 4; if any needle fires, that is a live defect — fix the lowering
   (not the needle list) and say so in the summary.
4. Write test 5 in the BigQuery conformance binary.
5. Retire the stale claims in `crates/smelt-cli/tests/maintenance_conformance_bigquery/`:
   `main.rs`'s doc comment (still describes the 2026-08-17 7/21 run as current),
   `dags_bigquery.rs` and `gate_composed_bigquery.rs` (the latter still says the fix "has not
   yet been re-confirmed with a live re-run"). Replace each with the table's row plus the
   2026-08-22 sweep result and the re-sweep debt.
6. Apply the spec delta.
7. Add a §"Criterion 6 — the two known live conformance failures" section to
   `docs/handoffs/2026-09-08-github-activity-findings.md` carrying the table verbatim, the
   offline gates that now hold each mechanism, and the HEAD-re-sweep debt with its date.
8. Append the phase-9 decision-log entry to the outcome and write `phases/09-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-maintenance-testkit --test googlesql_render`
- `cargo test -p smelt-dialect --test modulo_lowering --test power_lowering`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — the same two
  families' DuckDB leg at HEAD (the strongest engine-independent evidence available offline)
- `cargo check -p smelt-cli --features bigquery --tests`, then
  `cargo test -p smelt-cli --features bigquery --test maintenance_conformance_bigquery
  bigquery_oracle_relation` with `SMELT_BQ_PROJECT` unset (must run, not skip)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`test(bigquery): gate the two characterised conformance failures offline`
