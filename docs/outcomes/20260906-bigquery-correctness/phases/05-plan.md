# Phase 5 plan — punch-list 2b: make the enrichment-keyed cell live on the run path

## Objective

Phase 4 derives an `UpstreamMutation(gold.repo_dim)` / `Technique::ColumnScopedMerge` cell
for `gold.events_enriched`, but nothing on the run path can see it: both
`resolve_live_column_scoped_cell` call sites pass no model edges, so the resolver's
`derive_resolved` never learns the edge exists. This phase threads edges into the resolver
and gives the resolved cell a real dispatch whose write scope is the whole table (not the
run window), so `github_activity`'s `current_repo_name` actually heals and the
`gold_events_enriched` stale-row count reaches zero. Advances success criteria 3 (a gate
that would have caught it) and 5 (the spine's first registered divergence resolved rather
than tolerated).

## The design decision this phase settles

An enrichment-keyed cell's write is addressed by the **join key** (`repo_id`), not by a
partition interval — `admit_enrichment_keyed_merge` says so explicitly
(`PartitionLocal::No { why: "addressed by its own join key, not a partition interval" }`,
`scans: vec![]`). The existing per-batch `ColumnMergeDispatch::Full` arm MERGEs
`compiled.sql` *already filtered to the batch's `[start, end)` window*, which would heal
only rows this run's window rewrote — the replay's day-N run would never revisit day N−3's
rows, and the stale count would stay non-zero. So an EnrichmentKeyed cell is **excluded**
from the per-batch dispatch and dispatched once per run, after the batch loop, over the
model's **unwindowed** compiled SQL, updating only the cell's own group columns. The
edge's declared `allow_full_scan: true` is what licenses that read.

## Spec delta (first)

`docs/specs/incremental_models.md` §"Upstream model edges" — phase 4 added the
enrichment-keyed route's *derivation*; add its **dispatch**: an enrichment-keyed cell runs
once per run after the model's own creation-trigger writes, never per batch; its read is
the model's whole compiled output (licensed by `allow_full_scan`), its write a keyed MERGE
on the downstream's own write key updating only the cell's group columns; it never fires on
the creation run. Same section: a model-edge `UpstreamMutation` trigger has no
`SourceInfo` and therefore no recorded source-mutation baseline, so §"When a mutation cell
dispatches"'s fingerprint gate **fails open to dispatch** for it — stated as the declared
behaviour with its cost (a full-table merge every run), not left as an accident of a
lookup returning `None`.

## Tests (red-green)

1. `crates/smelt-runtime/tests/model_edge_creation_cell.rs` ::
   `resolve_live_column_scoped_cell_sees_an_enrichment_keyed_edge_cell` — with `model_edges`
   supplied, the resolver returns a cell whose trigger source is the *edge* name and whose
   `key_scope.discovery` is `EnrichmentKeyed`; with the same inputs and an empty edge slice
   it returns `None`. (Red today: the resolver takes no edges at all.)
2. same file :: `an_enrichment_keyed_cell_is_excluded_from_the_window_scoped_dispatch` — the
   per-batch decision returns `None` for an `EnrichmentKeyed` cell, so no batch ever
   window-scopes it.
3. same file :: `a_model_edge_trigger_has_no_mutation_baseline_and_fails_open` —
   `resolve_upstream_mutation_gate` returns `None` for an edge name (no `SourceInfo`) and
   the caller's `mutation_should_dispatch` is therefore `true`.
4. same file :: `dimension_unique_key_for_a_model_edge_comes_from_the_edge` — the
   unique-key lookup resolves an edge trigger from `ModelEdge::unique_key`, not from an
   absent `source_infos` row.
5. `crates/smelt-runtime/tests/statement_parity/` (new case) — the heal's executed
   statement group is the single-owner emitter's output
   (`emit_column_scoped_merge{,_suppressed}`) over **only** the cell's group columns, and
   its source SELECT carries no `[start, end)` predicate.
6. `crates/smelt-cli/tests/github_activity_replay.rs` ::
   `enrichment_heal_repairs_rows_written_before_the_rename` — after the 30-day replay, every
   `gold_events_enriched` row's `current_repo_name` equals its repo's current
   `gold_repo_dim` value; zero stale rows, including rows written days before the rename.
7. `crates/smelt-cli/tests/github_activity_oracle.rs` — delete the `gold_events_enriched`
   `DIVERGENCE_REGISTRY` entry (and the two tests that assert its staleness is *present*:
   `enrichment_staleness_is_confined_to_the_enriched_column`,
   `enrichment_staleness_is_never_a_fabricated_value`), replacing them with
   `gold_events_enriched_matches_the_full_refresh_oracle`. With the entry gone the existing
   unregistered-divergence sweep enforces exact equality — that is the gate.

## Tasks

1. Make the spec edit above.
2. Add `model_edges: &[ModelEdge]` to `resolve_live_column_scoped_cell`
   (`crates/smelt-runtime/src/maintenance_driver/resolve/live_cells.rs`); switch it to
   `maintenance_availability::derive_resolved_with_edges`.
3. Replace its `for source in explicitly_mutable` loop with an iteration over the derived
   plan's own distinct `Trigger::UpstreamMutation` source names in cell order — behaviour-
   preserving for declared sources, and the only way an edge trigger is reachable.
   Deterministic order (today's `HashSet` iteration is not).
4. Pass the edge list at both call sites in `crates/smelt-runtime/src/execute/project/mod.rs`:
   the non-keyed branch's `model_edges` (already built at ~L2587), and the keyed branch's
   `keyed_model_edges` — **hoist** its construction (~L1423) above the
   `resolve_live_column_scoped_cell` call at ~L1343.
5. Exclude `EnrichmentKeyed` cells from the window-scoped `column_merge_dispatch` in both
   branches (test 2).
6. New module `crates/smelt-runtime/src/execute/enrichment_heal.rs` with a single
   `execute_enrichment_keyed_heal(...)`: no-op unless the cell is `EnrichmentKeyed` and the
   target table existed before the run; compiles the model's SQL unwindowed via
   `compiler.compile_with_sql_and_ephemerals`; calls
   `maintenance_driver::execute_column_scoped_merge_full` with the model's own write key
   (`inc_plan.config.unique_key` / the keyed branch's `unique_key`, which `merge_key:` folds
   into), the cell's **group** columns, and the already-resolved `WriteSuppression`. An empty
   write key keeps `decide_column_merge_dispatch`'s documented no-error posture — fall back,
   but `tracing::warn!` naming the model and edge rather than vanishing.
7. Call it from both branches — non-keyed: after the batch loop, before the
   `record_upstream_mutation_baseline` block (~L3878); keyed: after the fold, alongside the
   existing `column_scoped_cell` dispatch. Set `used_column_scoped_merge` so the manifest
   strategy label reports it. Skipped when `key_edge_dispatch` took the run (mutually
   exclusive by construction; the existing same-trigger bail stays).
8. Make the dimension-`unique_key` lookup edge-aware via a small helper falling back to
   `ModelEdge::unique_key` (test 4); document at the mutation-gate call site that an edge
   trigger fails open to dispatch, per the spec edit (test 3).
9. Update `examples/github_activity/models/gold/events_enriched.sql`'s header comment: the
   cell is now live, phase 5's forward reference removed.
10. `crates/smelt-runtime/src/execute/project/mod.rs` sits exactly at its
    `.claude/large-file-baseline.txt` entry (4691). Keep it net-neutral or smaller: the new
    call sites are one helper invocation each, and if it still grows, move the
    `column_scoped_cell` resolution block into `enrichment_heal.rs`'s module rather than
    raising the baseline.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test model_edge_creation_cell --test statement_parity --test execute_parity --test availability_seam`
- `cargo test -p smelt-cli --test github_activity_replay --test github_activity_oracle --features duckdb` (the 30-day replay is slow; run it to completion, it is this phase's real gate)
- `cargo test -p smelt-logical --test keyed_model_edge --test model_edge_enrichment_mutation`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`fix(maintenance): dispatch the enrichment-keyed model-edge cell on the run path`
