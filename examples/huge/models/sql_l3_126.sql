---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    platform,
    order_id
FROM smelt.ref('py_l2_338')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_189') WHERE is_active = true
)
