---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    category,
    amount,
    ROW_NUMBER() OVER (PARTITION BY category ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l1_126')
