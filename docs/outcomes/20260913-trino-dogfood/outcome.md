# Outcome: The GitHub-activity pipeline runs on Trino and DuckDB, and the numbers agree

**Created:** 2026-09-13
**Status:** queued
**Driver:** loop. Docker only — unlike the BigQuery and Databricks dogfood spines, there is no
cloud account, no credential to mint, no human provisioning gate and no bill, so this outcome has
no split driver. Live-tier phases must emit `<<PHASE_BLOCKED>>` when the coordinator is
unreachable, never skip green.
**Depends on:** T1–T4 (`20260913-trino-target-spine`, `-trino-emission`, `-trino-ledger`,
`-trino-incremental`). This is the outcome that proves the previous four on a real pipeline over
real rows.
**Source:** T5 of the five-outcome Trino programme agreed 2026-09-13. Direct repeat of
`docs/outcomes/20260906-bigquery-dogfood-spine` and
`docs/outcomes/20260912-databricks-dogfood-spine` on a fourth target, reusing
`crates/smelt-cli/tests/dual_target_harness.rs` rather than duplicating it.
**Spec anchors:** `docs/specs/multi_backend.md` §"Parity contract", §"Loading data into a
backend", §Known Divergences; `docs/specs/sources.md`; `docs/specs/incremental_models.md`
§"The equivalence invariant"; `docs/specs/run_state.md`

## The outcome

`examples/github_activity/` — the bronze→silver→gold→mart pipeline already trusted on DuckDB,
BigQuery and Databricks — runs to completion on Trino over Iceberg, and its numbers agree with
DuckDB's row for row.

The population is the **same** stable 0.1% GitHub Archive sample the other targets see. A loader
replays the committed Parquet fixture day by day into an Iceberg table through the backend's own
load path, including the deliberate 2% previous-day redelivery, so the targets are comparable
row for row rather than merely both plausible. Against that table the pipeline runs a full
refresh and then at least three consecutive incremental windows. Every model's output equals
DuckDB's over the same rows or is a **registered** divergence with a reason; every model's
incremental state after every window equals a full-refresh oracle written to a sibling schema.

This is where the four preceding outcomes stop being claims. A capability flag measured in T1, a
verdict probed in T2, a residency decision made in T3 and a family proved generatively in T4 all
meet a real pipeline with real skew, real late arrivals and real redelivery. The defects that
surfaces are **written down, not fixed** — the findings handoff is the deliverable, and it is the
input to a later `trino-correctness` outcome that this outcome does not create.

## Success criteria (checkable)

1. **The loader lands real days through the backend's own path.** A loader script replays
   `examples/github_activity/seeds/github_events_sample.parquet` one day at a time into the
   Trino target's Iceberg table, applying exactly the redelivery rule of
   `examples/github_activity/load_day.sh` and stamping `ingested_date`. It goes through the
   backend's own load path — never a host filesystem path the coordinator cannot see. A per-PR
   test proves its per-day slice is row-identical to the DuckDB loader's for every day of the
   fixture, and that it is idempotent per day. The loader is documented as **external to smelt**;
   smelt's source declaration is the contract.
2. **Loaded, verified against the fixture's own counts.** At least the fixture's full day range
   is present, `ingested_date` stamped and the redelivered slice included, verified by row counts
   against the Parquet fixture's own counts plus the expected 2%.
3. **The Trino leg runs.** The same model set compiles and runs against the Trino dogfood schema
   — a full refresh, then **at least three consecutive incremental windows** — with the run
   report captured for each. Every compile refusal and runtime failure is *recorded*, not fixed
   in place, unless the fix is the only way a run completes at all.
4. **The two targets agree.** `crates/smelt-cli/tests/dual_target_harness.rs` gains a Trino leg
   **generalised over the target rather than duplicated** — it already carries BigQuery and
   Databricks legs, so a fourth copy would be the wrong answer. Each model's output over the same
   rows is equal between DuckDB and Trino, or the difference is a registered divergence with a
   reason. An unregistered difference fails.
5. **The numbers are trustworthy on Trino itself.** After each incremental window, each model's
   state on Trino equals a full refresh over the inputs seen so far, written to a sibling
   `_oracle` schema — the equivalence invariant on a real pipeline rather than on generated
   recipes.
6. **Cost and shape are measured, not guessed.** Wall time per window, per-model execution time,
   and the coordinator/worker resource the run actually consumed are recorded — the analogue of
   the BigQuery spine's cost accounting, which is what made its coarse-vs-fine window comparison
   possible. Whether a wider run window changes the result (it must not) or the cost (it may) is
   measured on at least one pair of schedules.
7. **Evidence banked.** `docs/handoffs/2026-XX-XX-trino-findings.md` lists every defect,
   divergence, missing emission verdict, unsupported construct and downgraded cell the live runs
   surfaced, each with the model and statement that provoked it, plus the Iceberg/Trino
   constraints that shaped the design. `multi_backend.md` §Known Divergences is updated from it.
   This document is the input to a follow-on `trino-correctness` outcome which this outcome does
   **not** create.
8. **Documented for a user.** `docs-site/` gains the Trino target page a reader can follow from
   an empty directory to a running pipeline: the compose tier, the target block, the source
   declaration, and the capability differences that will bite them (no `QUALIFY`, no `::`, and
   whatever else T1–T4 measured).
9. **Gates green, and the other tiers unharmed.** `verify-phase.sh` passes; no ratchet lowered;
   the DuckDB, Spark, BigQuery and Databricks parity tiers still pass unchanged, and the Trino
   compose tier coexists with the Spark container and its warehouse without a port, name or path
   collision.

## Out of scope

- **Fixing** anything the live runs surface, beyond what is needed to make a run complete at all.
  Fixes belong to a follow-on `trino-correctness` outcome scaffolded from criterion 7's handoff.
- **Unattended/scheduled execution.** Trino here is self-hosted in Docker on this machine, so the
  Databricks spine's criterion 11 (a platform-scheduled daily job with run state off the laptop)
  has no honest analogue: a compose-resident scheduler would be rented infrastructure proving
  nothing about smelt. Deliberately excluded, per the build-vs-rent boundary.
- **A second connector or a federated pipeline** — no model reads from two Trino catalogs at
  once, tempting as Trino makes it.
- **Cross-engine data exchange with Trino** (Trino writing Parquet that DuckDB reads directly, or
  the reverse) as a distributed-execution route. Parity here is compared, not fused.
- **Expanding the fixture population or changing the sample.** The committed fixture is the
  population, unchanged, precisely so the targets stay comparable.
- **Performance tuning** of either the pipeline or the tier. Criterion 6 measures; it does not
  optimise.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | The loader, proved offline: day-by-day replay of the committed fixture into Iceberg through the backend's own load path with the 2% redelivery and `ingested_date` stamp, plus the per-PR test proving each day's slice is row-identical to the DuckDB loader's and idempotent | pending |
| 2 | Load the fixture's full day range into the dogfood schema and verify row counts against the fixture's own counts plus the expected 2% | pending |
| 3 | First live Trino run: full refresh of the whole model set; record every compile refusal and runtime failure rather than fixing them in place | pending |
| 4 | Three or more consecutive incremental windows, run reports captured, frontier and engine-resident state inspected between runs | pending |
| 5 | Dual-target parity: a Trino leg of `dual_target_harness.rs` generalised over the target, every model compared to DuckDB over the same rows, each difference registered with a reason or failing | pending |
| 6 | Trust the numbers on Trino: full-refresh oracle in a sibling `_oracle` schema versus incremental state after each window | pending |
| 7 | Measure cost and schedule sensitivity: per-window and per-model timings, resource consumed, and one coarse-versus-fine schedule pair proving the result is identical while the cost differs | pending |
| 8 | Bank the evidence: `docs/handoffs/2026-XX-XX-trino-findings.md` with every defect, divergence, unsupported construct and downgraded cell, each attributed to the model and statement that provoked it | pending |
| 9 | Close: the `docs-site/` Trino target page runnable from an empty directory, §Known Divergences rewritten from the handoff, all four other target tiers re-verified unchanged, `verify-phase.sh` green | pending |

## Decision log

- **2026-09-14 — hand-forward from `20260913-trino-target-spine` phase 11.** Two measured gaps
  for this outcome to route around or close: a model projecting a Trino `array(...)` result
  column does not decode to Arrow yet — the HTTP statement client's result-page decoder has no
  arm for the JSON shape Trino's `/v1/statement` protocol uses for array cells
  (`docs/specs/multi_backend.md` §Known Divergences), so an array-typed projection will fail a
  dogfood run rather than silently misread it; and `print_body_for_dialect` has an
  `unimplemented!()` reachable path for some construct on `SqlDialect::Trino` not yet exercised
  by T1's narrower fixture set — expect it to surface on a wider real pipeline before this
  outcome's phase 1 finishes model selection.

## Blocked
