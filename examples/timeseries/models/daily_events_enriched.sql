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
  # row — the row identity the regular incremental execution loop's
  # column-scoped `MERGE` (MP11) keys on when a `raw.users` mutation drives
  # the `{user_name}` cell, in place of the default region-recompute
  # `DELETE`+`INSERT`.
  unique_key: [event_id]
maintenance:
  scan_bounds:
    per_source:
      raw.users:
        allow_full_scan: true
---
-- Fact (events) enriched with the dimension (users) — the fact+dimension
-- enrichment shape MP11 wires a live column-scoped MERGE for
-- (docs/specs/maintenance_plan.md §"Per-cell admission"; the
-- `smelt-runtime::maintenance_driver` "first live cell" story). `raw.users`
-- is an unclocked, explicitly `mutation_profile: mutable_snapshot`
-- dimension: renaming a user broadcasts to every fact row that references
-- them — the `{user_name}` column group's `UpstreamMutation` cell.
SELECT
    e.event_id,
    date_trunc('day', e.event_timestamp) AS event_date,
    e.user_id,
    e.event_type,
    u.user_name
FROM smelt.sources.raw.events e
JOIN smelt.sources.raw.users u ON e.user_id = u.user_id
