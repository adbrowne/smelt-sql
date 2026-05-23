---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    quantity,
    ROW_NUMBER() OVER (PARTITION BY status ORDER BY created_at) AS win_val
FROM smelt.sessions
