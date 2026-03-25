---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    status,
    created_at
FROM smelt.ref('py_l3_409')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_409') WHERE quantity > 0
)
