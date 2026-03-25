---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    ip_address,
    category,
    quantity,
    segment
FROM smelt.ref('sql_l1_29')
WHERE category IS NOT NULL
