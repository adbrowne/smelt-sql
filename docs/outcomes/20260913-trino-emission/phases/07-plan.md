# Phase 7 — `Restructure`/`Rewrite` on a fourth dialect

## Objective

Give Trino explicit, live-measured verdicts everywhere it offers a built-in only in the
*opposite* call position from the one an author may write: an aggregate-position spelling,
`Emission::Restructure(RestructureId::WindowToCte)` at `Position::WholePartitionWindow`, and
an `Emission::Unsupported` running-frame refusal at `Position::Window`. Advances criteria 10
(the restructure works on Trino, null-safe join in Trino's spelling, running-frame refusal),
2 (the `FIRST`/`LAST`-family and percentile-family verdicts become stated rather than gaps)
and 5 (`dialect_gaps_trino` ratchets **down** as those ledger rows close).

## Measurement comes first (the phase's own gate)

Nothing below is written before it is measured. Bring the tier up
(`bash scripts/trino-up.sh` + `source scripts/trino-env.sh`) and settle, for each candidate,
all three positions — plain aggregate, `OVER (PARTITION BY g)`, and `OVER (PARTITION BY g
ORDER BY t)`:

- `max_by(x, t)` / `min_by(x, t)` (smelt `ARG_MAX`/`ARG_MIN`, today `gap(…, "#209")`)
- `approx_distinct(x)` (smelt `APPROX_COUNT_DISTINCT`, today a `#209` gap)
- `percentile_cont(f) WITHIN GROUP (ORDER BY x)` / `percentile_disc` (today a `#209` gap
  whose recorded reason is *shape*, `WITHIN GROUP` mandatory — which is smelt's own spelling,
  so aggregate position is very likely already correct and only the window form is missing)
- whether **any** smelt built-in is analytic-*only* on Trino, i.e. whether `AnalyticToCte`
  applies here at all

If the tier cannot be brought up this phase blocks — these are the never-skip-green legs.
Record every measured answer in the summary; a candidate that turns out to be `Native` in
every position gets a stated `Native` verdict with the measurement date, not silence.

## Spec delta (first commit hunk, after measurement)

`docs/specs/multi_backend.md` §"Statement-level lowering": the paragraph beginning "**An
aggregate-only built-in in window position.**" enumerates GoogleSQL's and DuckDB/Spark's
cases — add Trino's measured set to that enumeration, and to the null-safe-spelling bullet
add Trino's `IS NOT DISTINCT FROM` (already `BackendCapabilities::trino_iceberg()` data, live
probed in `capability_probes.rs`). If the measurement finds no analytic-only built-in on
Trino, say so in one sentence there rather than leaving `AnalyticToCte`'s dialect list to be
read as exhaustive by omission.

## Tests (red first)

1. `smelt-dialect/tests/window_decorrelation.rs::trino_null_safe_join_spelling` — the Trino
   restructure prints `IS NOT DISTINCT FROM`, never `<=>`; sibling of the existing
   `null_safe_join_spelling_per_backend`.
2. `smelt-dialect/tests/window_decorrelation.rs::trino_no_partition_by_uses_cross_join` — the
   degenerate one-row-CTE shape holds on the fourth dialect too.
3. `smelt-dialect/tests/restructure_plan.rs::trino_window_to_cte_planned_from_source_cst` —
   a restructured model whose body also contains `^` (which Trino prints as `POWER(...)`, and
   which does not parse back as smelt's operator) still plans correctly, proving the plan is
   built from the source CST, not from printed SQL.
4. `smelt-dialect/tests/unsupported_emission.rs::trino_running_frame_refused` — a running
   window (`ORDER BY t`) over each measured aggregate-only built-in fails with
   `UnsupportedOnBackend`, the message naming the built-in, Trino, and the whole-partition
   requirement.
5. `smelt-types/tests/registry_coverage.rs::trino_restructure_pairs_with_a_window_refusal` —
   structural: every Trino `Restructure` verdict sits at `WholePartitionWindow` and is
   accompanied by a stated `Position::Window` verdict (so a new one cannot fall through to
   the implicit `Native`).
6. `smelt-db/tests/dialect_audit/trino.rs::trino_restructured_window_agrees_with_duckdb` —
   live: compile one model using the built-in over `OVER (PARTITION BY g)` against a fixture
   whose rows include a **NULL** `g`, execute the printed Trino SQL, and assert both the row
   count and the per-row values equal DuckDB's native-window answer. This is the assertion
   `restructure_multiplicity.rs` makes for DuckDB, carried to live Trino.
7. `smelt-db/tests/dialect_audit` existing legs (`schema_leg_trino`, `value_leg_trino`,
   `ledger_gates`) stay green with the closed gap rows removed — the two-sided rule *forces*
   their removal, so leaving one behind is a failure, not a tidy-up.

## Tasks

1. Bring the tier up; run the measurement sweep above; write the answers down.
2. Apply the spec delta.
3. State the measured verdicts in `crates/smelt-types/src/signatures/builtins/
   extended_aggregates.rs`, each with a dated `measured live` comment, following the existing
   BigQuery `ARG_MAX` triple as the shape (aggregate `Rename`/`Native`,
   `WholePartitionWindow` → `Restructure(WindowToCte)`, `Window` → `Unsupported { reason }`).
4. Write tests 1–5 red, then green. No printer change should be needed; if one looks needed,
   it is a `BackendCapabilities` flag or `Signature::emission` data instead —
   `emission_ownership` is the oracle.
5. Delete the `#209` ledger rows the new verdicts close; lower `dialect_gaps_trino` in
   `.claude/dialect-gaps-baseline.txt` by exactly that count (down only).
6. Write test 6 and run it live.
7. Regenerate `docs/reference/dialect-coverage.md` (`SMELT_REGEN_DOCS=1`) if the doc-sync gate
   asks; leave the §Known Divergences rewrite to phase 10.
8. Write `phases/07-summary.md`, including every measured answer and the
   `AnalyticToCte`-applies-or-not ruling.

## Verification

- Live (tier exported): `cargo test -p smelt-db --test dialect_audit --quiet`,
  `cargo test -p smelt-backend-trino --quiet`.
- Offline (tier unexported): `cargo test -p smelt-dialect --test window_decorrelation --test
  restructure_plan --test unsupported_emission --test emission_ownership`;
  `cargo test -p smelt-types --test registry_coverage`;
  `cargo test -p smelt-runtime --test restructure_multiplicity`.
- `bash .claude/scripts/verify-phase.sh` with the Trino tier **unexported** (phases 5/6's
  documented workaround for the shared-catalog concurrency collision).
- No ratchet raised: `dialect_gaps_trino` moves down only; registry-migration and parser-gaps
  baselines untouched.

## Commit message

`feat(dialect): give Trino stated position-opposite verdicts and a live restructure leg`
