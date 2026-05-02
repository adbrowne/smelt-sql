---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    os_name,
    event_time,
    ROW_NUMBER() OVER (PARTITION BY os_name ORDER BY created_at) AS win_val
FROM smelt.models.sql_l1_137

