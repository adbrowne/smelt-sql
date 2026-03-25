---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    ip_address,
    event_date,
    cohort_date
FROM smelt.ref('sql_l1_155')
WHERE event_type = 'purchase'
