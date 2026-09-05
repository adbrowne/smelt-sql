# Ten directions for smelt

**Date:** 2026-09-05
**Status:** brainstorm — a list of candidate directions, not a commitment to any
**Author:** Andrew Browne, with Claude

## Purpose

A short list of things smelt could do next, or that could be built with it,
grounded in where the project actually stands today (maintenance-plan
substrate, conformance gates, dialect registry, Maintenance Atlas, LSP).
Each entry says what it is and why it fits. The final section picks one to
start on.

## The list

1. **Maintenance Atlas as a public playground.** Turn the plan-prover
   explorer into a hosted page where anyone pastes a model and sees which
   proofs hold, which technique is assigned, and why. It is the single best
   demo of the logical/physical split and needs no warehouse.

2. **Property-based fuzzing of the equivalence invariant against Spark and
   BigQuery.** The maintenance conformance gate
   (`cargo test -p smelt-cli --test maintenance_conformance`) only runs on
   DuckDB. Widening the recipe pool to the other two backends turns the
   strongest correctness claim in the project into a cross-engine one.

3. **A dbt adapter mode.** `docs/research/` from May already sketched it:
   read a dbt manifest, treat naive full-refresh models as smelt models, and
   let the maintenance plan incrementalize them with a proof attached. It is
   the lowest-friction adoption path for people who already have a dbt
   project.

4. **"Explain the diff" for model edits.** Given two versions of a model,
   report what changed in the derived properties: grain lost, bound widened,
   technique downgraded from keyed to full refresh. Surface it in the LSP as
   a code lens and in CI as a PR comment.

5. **Self-directed scheduler as a daemon.** The research doc
   (`20260905-self-directed-scheduler.md`) is written. Build it as a
   long-running process that watches source freshness and runs only the
   models whose deltas are ready, with the ledger as its only state. Pair it
   with a small status TUI.

6. **A mutation-testing leaderboard for the gates.** A cargo-mutants campaign
   has already been run once. Publish per-gate kill rates in a generated docs
   page, and make the ratchet two-sided so a gate whose kill rate falls fails
   CI. That turns "our gates are strong" into a measured claim.

7. **Cost-aware planner rules.** Use the engine's row estimates plus the
   maintenance plan to pick between full refresh and incremental per cell on
   cost, not just admissibility. Expose the decision in explain output so an
   engineer can override it.

8. **Cross-engine "spill" execution.** Small models run in DuckDB, large ones
   in Spark, with Parquet as the exchange format (Spark writes, DuckDB reads;
   no copy step). A single pipeline that provably straddles two engines is
   the multi-backend pitch made concrete.

9. **A conformance corpus for other tools.** The typed model recipes and the
   full-refresh oracle are reusable. Package them as a standalone harness that
   scores any incremental tool, including dbt and SQLMesh, on the same
   equivalence invariant.

10. **A web-analytics tutorial that ends in a live dashboard.** The
    seven-page series already exists. Extend it with a final chapter where
    the pipeline feeds a shareable dashboard, so a reader finishes with
    something they can show rather than a terminal transcript.

## Recommendation

Start with **4, "explain the diff"**. It reuses the property-composition walk
and the maintenance plan directly, ships inside the LSP that already exists,
and makes the proofs visible at the moment someone edits a model. Planning
for it follows the usual spec → plan route.
