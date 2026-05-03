---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    updated_at,
    ROW_NUMBER() OVER (PARTITION BY plan_type ORDER BY created_at) AS win_val
FROM smelt.sql_l2_43

