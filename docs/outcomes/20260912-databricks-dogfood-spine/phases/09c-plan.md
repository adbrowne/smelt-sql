# Phase 9c plan — close criterion 8, root-cause the Databricks succession fold, offline

## Objective

Finish criterion 8 by registering the one understood violation the committed equivalence report
holds and landing 9b's two deferred report-driven gates. Then root-cause criterion 7's blocker —
`silver_actor_naming`'s 520 extra rows on Databricks — from the maintenance-plan and
statement differential `smelt explain` derives with **no workspace and no token**, and land the
resolution. No live Databricks call in this phase; the re-run is row 9d.

## The hypothesis this phase tests first (cheapest decisive experiment)

Both Databricks succession paths fold `(key, clock)` by construction
(`emit_succession_patch`'s `__smelt_dedup` CTE; `emit_succession_full_rebuild`'s
`ROW_NUMBER() … = 1`), and the fold is normative (`docs/specs/incremental_shapes.md`
§`SuccessionClockTie`: "identical rows are a redelivery and fold once"). Databricks' 26,220 rows
is *exactly* the source row count — no fold at all — and the 9b oracle leg shows the full
refresh carries it too. So the likeliest root cause is that on the `databricks` target
`silver.actor_naming` is **not assigned the succession-patch technique** (a capability clamp or
downgrade picks a plain rebuild/insert instead), rather than a defect inside either emitter.
That is pure plan data, so `smelt explain --target dev` vs `--target databricks` settles it
offline (`docs/specs/cli.md`: explain reads target metadata, never a connection).

If the differential shows the *same* technique and statements on both targets, the fold is
emitted and Databricks is not honouring it; in that case reproduce against local Spark+Delta
(`scripts/spark-up.sh`, `source scripts/spark-env.sh`) — still no Databricks workspace — and
root-cause there. Either way the conclusion, not a guess, picks the route from `## Blocked`.

## Spec delta

Only if the differential shows the clamp is *intended* behaviour for the Databricks capability
profile — then `docs/specs/incremental_shapes.md` §"The succession grain" (the
backend-portability paragraph around `supports_qualify`) must say which backends can carry the
succession grain and what happens on one that cannot, and `docs/specs/multi_backend.md`
§Surface's capability table must carry the flag. A missing fold that no spec licenses is a
defect and needs no spec edit.

## Tests (red-green)

1. `github_activity_dbx_oracle::the_committed_equivalence_report_shows_no_violation` — 9b's
   deferred gate; red until `gold_events_enriched` is registered, then green over the committed
   `09b-equivalence.json`.
2. `github_activity_dbx_oracle::equivalence_registry_entries_are_all_live` — the two-sided
   ratchet: every `EQUIVALENCE_DIVERGENCE_REGISTRY` entry must name a relation the committed
   report actually shows diverging, so a healed divergence forces the entry out.
3. `github_activity_dbx_oracle::the_equivalence_sweep_fails_closed_on_an_empty_registry` —
   confirm the existing fail-closed control still drives the real path now that the registry is
   non-empty (mirror the BigQuery suite's control, do not weaken it).
4. A new offline differential test (place beside the existing maintenance-plan explain tests,
   `crates/smelt-cli/tests/explain_maintenance/`): for `silver.actor_naming` in
   `examples/github_activity/`, the technique assigned on `--target databricks` equals the one
   assigned on `--target dev`, and the emitted statement set contains the `(key, clock)` fold.
   This is the red test for the root cause and the standing gate against its regression.
5. Whatever test the confirmed root cause demands — a `smelt-logical` emitter unit test if the
   fold is mis-emitted for the Spark dialect, or a planner/clamp test if the technique
   assignment diverges. Named in the summary, red before the fix.

## Tasks

1. Add `gold_events_enriched` to `EQUIVALENCE_DIVERGENCE_REGISTRY` in
   `crates/smelt-cli/tests/github_activity_dbx_oracle.rs` with the `UnorderedColumnDivergence`
   bound on `current_repo_name` and a reason naming phase 7b's `EnrichmentKeyed`→`MergeLedger`
   downgrade and the unwindowed heal it sacrifices — matching the dual-target suite's entry and
   meeting that registry's higher bar (a licence to depart from the equivalence invariant).
2. Land tests 1-3; confirm 17/17 on `cargo test -p smelt-cli --test github_activity_dbx_oracle`.
3. Run the offline differential by hand first: `smelt explain --target dev --json` and
   `--target databricks --json` over `examples/github_activity/`, plus `--show-sql`, and diff
   `silver.actor_naming`'s technique and statements. Record the finding verbatim in the summary.
4. Write test 4 as the standing encoding of that differential (red if step 3 found a
   divergence).
5. If step 3 found no plan-level divergence, reproduce against local Spark+Delta and narrow to
   the statement that loses the fold; tear the server down afterwards.
6. Land the fix at its single owner — a maintenance emitter in `smelt-logical` (never a printer
   branch, never a backend authoring a statement) or the clamp/technique-assignment path in
   `smelt-logical`'s plan derivation — with test 5 red first.
7. Record in `phases/09c-summary.md`: the corrected inference, the measured differential, the
   route taken from `## Blocked`, and exactly what 9d must re-run (parity sweep only, or parity
   plus a refreshed oracle sweep if a maintenance statement changed).
8. If the root cause turns out to need a live Databricks observation the differential cannot
   supply, do **not** burn the phase: commit the criterion-8 half plus the evidence, append to
   `## Blocked`, and emit `<<PHASE_BLOCKED>>` naming the live observation 9d must make.

## Verification

- `bash .claude/scripts/verify-phase.sh` — fmt, clippy both feature sets, shellcheck, full
  `cargo test`, `example_diagnostics`.
- `cargo test -p smelt-cli --test github_activity_dbx_oracle` — all green, registry non-empty.
- `cargo test -p smelt-cli --test github_activity_dual_target` — unchanged (23/23); the
  Databricks leg stays deferred until 9d.
- `cargo test -p smelt-runtime --test statement_parity` and
  `cargo test -p smelt-logical --test walk_coverage` — the maintenance-plan/statement
  single-ownership gates, if task 6 touches an emitter.
- `cargo test -p smelt-cli --test maintenance_conformance` — the equivalence-invariant
  generative gate, if task 6 touches the succession fold.

## Commit message

`outcome(databricks-dogfood-spine): phase 9c closes criterion 8 and root-causes the Databricks succession fold offline`
