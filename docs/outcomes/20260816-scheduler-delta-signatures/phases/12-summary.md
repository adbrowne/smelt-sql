# Phase 12 summary — scheduler-driven keyed→partition recipes in the conformance suite

## Shipped

- `smelt_runtime::propagation_live::resolve_live_plan` (`crates/smelt-runtime/src/propagation_live.rs`):
  the single owner of `--since-upstream`'s live-plan sequence (derive which keys/diffs matter,
  read both live, fold into a `SinceUpstreamPlan`). `crates/smelt-cli/src/commands/run.rs::run_since_upstream`
  now delegates to it instead of hand-rolling the four-call sequence inline.
- `crates/smelt-runtime/tests/since_upstream_propagation.rs::resolve_live_plan_matches_hand_wired_sequence`
  (test 1): proves the extraction over a staged `dag_kpart_a -> dag_kpart_b` project.
- `crates/smelt-cli/tests/maintenance_conformance/dags.rs`: a shared `run_plan` helper (mirrors
  `run.rs`'s dispatch loop exactly — one request per `plan.runs` entry, the whole
  `keyed_restrictions_from_plan` map on every request) plus three new generative tests:
  - `keyed_partition_scheduler_sweep_matches_oracle` (tests 2+3): source-rooted — plans
    `--source dag_kpart_a` *before* `dag_kpart_a` reruns, drives exactly `plan.runs`, asserts
    oracle-equality and `dag_kpart_b`'s manifest strategy is `per_group_recompute`.
  - `keyed_partition_scheduler_sweep_from_model_upstream_matches_oracle` (tests 4+5):
    model-rooted — `dag_kpart_a` rebuilds first, then the live plan is resolved; asserts the
    resolved `keyed_restrictions_from_plan` entry for `dag_kpart_b` names exactly the case's
    `touched_ids` (criterion 2's value-level-discovery evidence), plus oracle-equality and strategy.
  - `keyed_partition_scheduler_sweep_leaves_untouched_rows_bit_identical` (test 6): the
    before/after snapshot pattern over the scheduler-driven repair.

## Decisions

- **Both scenarios' delta names `dag_kpart_a` (the maintained model), never the raw `events`
  source.** `dag_kpart_a` is `grain: key` and per `keyed_grain_model_never_derives_an_edge`
  never derives an inbound propagation edge from anything — a delta on the raw source informs
  nothing for this DAG shape at all (empirically verified: empty `dirty_set_report`). "Source-
  rooted" vs "model-rooted" (the plan's own vocabulary) turned out to mean *when* the plan is
  resolved relative to `dag_kpart_a`'s own rebuild, not *which* address is named — both name
  `dag_kpart_a`, differing only in ordering. This reading is consistent with `plan.runs` for
  `dag_kpart_b` in EITHER case: a non-keyed-grain downstream of an admitted keyed edge always
  widens to whole-table once its upstream is visited (`propagate_with_keys`'s
  `e.downstream_grain != PartitionGrain::Keyed` branch), regardless of whether the keyed seed
  itself resolved to real values or stayed empty.
- **A second no-op run seeds the group-grain sidecar before any live-seed read.** Reused the
  precedent from `key_addressed_model_edge_lowering.rs::resolve_keyed_seeds_reads_changed_keys_off_the_sidecar`
  (run 1 = create, doesn't seed; run 2 = first live dispatch, seeds transactionally) — without
  it, the live seed read has no prior snapshot to diff against.
- **`dag_kpart_a` is rebuilt via a plain `select: ["dag_kpart_a"]` request, never through
  `plan.runs`.** Grain:key models never appear in `plan.runs` (no inbound edge to seed one).
- Pin 3 did not fire: the model-rooted scenario's live seed always resolved
  `KeyValues::Resolved` naming exactly the touched ids across the deterministic sample — no
  `Unresolved` case was observed, so no weakened assertion was needed.

## For the next planner

- **Real regression found and fixed during implementation**: the initial `resolve_live_plan`
  extraction in `run.rs` kept the live-read `Backend` alive for the whole function (including
  through the later `execute_project` loop that opens its own connection to the same DuckDB
  file), which broke `since_upstream.rs::composed_model_address_landed_delta_propagates`
  (`gold` table never created — the execute loop's own backend open silently produced no
  writes under the lock contention). Fixed by re-scoping the live-read backend to a block that
  drops before the execute loop runs, matching the original (pre-extraction) code's own
  scoping. This is a sharp edge worth flagging generally: any future refactor of
  `run_since_upstream` that hoists the live-read backend out of its own scope risks the same
  silent-lock-contention failure — it does NOT surface as a DuckDB lock error, just as missing
  writes, so it is easy to miss without the `since_upstream` CLI-surface regression net.
- Row 13's close-out sweep should double check `incremental_models.md`'s scheduler-currency
  divergence bullet against this phase's evidence — criteria 1–3 now have real generative
  scheduler-path coverage, not just the plain-whole-project-build coverage
  `keyed_upstream_partition_downstream_matches_oracle` already provided.
- Nothing else was deferred; all six planned tests are green and this phase's own scope is
  fully delivered.

## Gates

- `cargo test -p smelt-runtime --quiet` — green
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — green (79 tests, +3 new)
- `cargo test -p smelt-cli --test since_upstream --quiet` — green (13 tests; caught+fixed the
  regression above)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace test,
  example_diagnostics)
