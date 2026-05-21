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
    segment,
    session_id,
    page_path,
    cost
FROM smelt.sql_l3_166
WHERE status = 'active'

