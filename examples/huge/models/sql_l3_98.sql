---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    is_verified,
    cost,
    region
FROM smelt.ref('py_l2_347')
WHERE quantity > 0
