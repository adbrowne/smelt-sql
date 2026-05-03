---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    updated_at,
    transaction_id,
    RANK() OVER (PARTITION BY updated_at ORDER BY created_at) AS win_val
FROM smelt.sql_l3_33

