---
materialization: table
refresh: incremental
grain: partition
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
batched:
  # `event_id` is the fact source's own declared `unique_key`
  # (`models/sources/raw/events.yml`) and, since this model neither
  # aggregates nor fans the join out, still uniquely identifies each output
  # row — the row identity the horizon-clamped column-scoped `MERGE` (MP11's
  # `PartitionLocal::Yes` corner) keys on when a `raw.user_status` mutation
  # drives the `{status}` cell.
  unique_key: [event_id]
---
-- Fact (events) enriched with a CLOCKED, mutable dimension
-- (raw.user_status) — the genuine scan-clamp corner (`PartitionLocal::Yes`)
-- MP11's horizon-clamped column-scoped MERGE (F15,
-- `smelt_runtime::maintenance_driver::execute_column_scoped_merge`) is wired
-- to dispatch through, distinct from `daily_events_enriched.sql`'s
-- accepted-full-scan corner (`PartitionLocal::No`, driven by the unclocked
-- `raw.users`). `raw.user_status` is clocked (`timeseries.partition_column:
-- changed_at`) and this join carries an explicit, derivable window
-- predicate on that column relative to the fact's own `event_timestamp` —
-- `link_source` (`smelt-logical::maintenance::derive`) links it to the
-- output partition axis via a genuine `ScanClamp` instead of falling back
-- to the accepted-full-scan corner.
SELECT
    e.event_id,
    date_trunc('day', e.event_timestamp) AS event_date,
    e.user_id,
    e.event_type,
    s.status
FROM smelt.sources.raw.events e
JOIN smelt.sources.raw.user_status s
  ON e.user_id = s.user_id
 AND s.changed_at BETWEEN e.event_timestamp - INTERVAL '1 day'
                       AND e.event_timestamp + INTERVAL '1 day'
