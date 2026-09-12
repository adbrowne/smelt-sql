---
materialization: table
refresh: incremental
---
-- Per-event repository-naming history, recognised as the succession grain
-- from this shape alone: no `grain:`, `unique_key:` or `timeseries:` is
-- declared, matching `examples/scd2_succession/models/customer_history.sql`'s
-- own precedent (`docs/specs/incremental_shapes.md` §"The succession grain").
--
-- One row per `(repo_id, created_at)` carrying the name in force at that
-- event — not one row per rename. "Keep only the rows where the name
-- changed" needs `LAG`, and the classifier admits exactly one *row-local*
-- pre-window filter, so that reduction happens downstream in
-- `marts.naming_history` instead
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/03-plan.md`).
--
-- Driven by `raw.github_events`, whose `timeseries.partition_column` equals
-- its `event_time_column` (`created_at`) — the event-time posture. The
-- loader's deliberate previous-day redelivery therefore lands in a *closed*
-- partition here, exercising the append-only probe's late-arrival
-- classification rather than `SourceMutationProfileViolated`.
--
-- `created_at` is projected verbatim (not aliased away) — the
-- succession-patch technique's tombstone ledger resolves the clock column's
-- type from the model's own output schema by name
-- (`crates/smelt-runtime/src/maintenance_driver/succession/execute.rs`), so
-- the clock column must survive under its source name in the output.
SELECT
    repo_id,
    repo_name,
    created_at,
    LEAD(created_at) OVER (PARTITION BY repo_id ORDER BY created_at) AS valid_to,
    LEAD(created_at) OVER (PARTITION BY repo_id ORDER BY created_at) IS NULL AS is_current
FROM smelt.sources.raw.github_events
