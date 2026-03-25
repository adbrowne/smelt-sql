---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    cohort_date,
    order_id
FROM smelt.ref('py_l2_383')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_367') WHERE amount > 0
)
