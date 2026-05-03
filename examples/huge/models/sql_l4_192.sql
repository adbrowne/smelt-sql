---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    revenue,
    RANK() OVER (PARTITION BY created_at ORDER BY created_at) AS win_val
FROM smelt.sql_l3_133

