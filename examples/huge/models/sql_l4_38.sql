---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    order_id,
    RANK() OVER (PARTITION BY user_id ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_10')
