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
    cohort_date,
    transaction_id,
    referrer
FROM smelt.sql_l1_80
WHERE created_at >= '2024-01-01'

