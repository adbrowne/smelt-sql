---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    transaction_id,
    RANK() OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.models.sql_l2_5

