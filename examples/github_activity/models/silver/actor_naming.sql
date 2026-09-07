---
materialization: table
refresh: incremental
---
-- Actor-login naming history — the same succession shape as
-- `repo_naming.sql`, on a different key (`actor_id`) and driven by the
-- **arrival-partitioned** twin source (`raw.github_events_arrival`,
-- `timeseries.partition_column: ingested_date` differs from
-- `event_time_column: created_at`). The loader's deliberate redelivery is
-- stamped with the *current* day's `ingested_date` there, so the same
-- duplicate rows land in the *open* partition on this posture instead of the
-- closed one `repo_naming` sees
-- (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/03-plan.md`).
--
-- One row per `(actor_id, created_at)` carrying the login in force at that
-- event; `marts.naming_history` derives the actual renames downstream.
--
-- `created_at` is projected verbatim, matching `repo_naming.sql` — the
-- succession-patch technique's tombstone ledger resolves the clock column's
-- type from the model's own output schema by name.
SELECT
    actor_id,
    actor_login,
    created_at,
    LEAD(created_at) OVER (PARTITION BY actor_id ORDER BY created_at) AS valid_to,
    LEAD(created_at) OVER (PARTITION BY actor_id ORDER BY created_at) IS NULL AS is_current
FROM smelt.sources.raw.github_events_arrival
