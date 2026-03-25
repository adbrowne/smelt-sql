---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    price,
    is_verified,
    platform
FROM smelt.ref('py_l2_460')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_375') WHERE quantity > 0
)
