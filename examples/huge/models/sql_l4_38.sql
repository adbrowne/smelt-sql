---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    user_id,
    order_id,
    RANK() OVER (PARTITION BY user_id ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_94')
