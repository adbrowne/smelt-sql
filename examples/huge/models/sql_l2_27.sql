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
    email_domain,
    discount,
    RANK() OVER (PARTITION BY email_domain ORDER BY created_at) AS win_val
FROM smelt.sql_l1_74
