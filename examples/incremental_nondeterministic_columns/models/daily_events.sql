---
materialization: table
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
refresh: batched
batched:
  nondeterministic_columns:
    - inserted_at
---

-- `inserted_at` is an audit stamp: NOW() is frozen once per run (pinned run
-- clock), so it varies only between separate `smelt run` invocations, never
-- within one. The deterministic columns (event_date, user_id, event_count)
-- must match a full-refresh rebuild for every partition.
SELECT
    event_date,
    user_id,
    COUNT(*) AS event_count,
    MAX(event_ts) AS last_event_ts,
    NOW() AS inserted_at
FROM smelt.sources.events
GROUP BY 1, 2
