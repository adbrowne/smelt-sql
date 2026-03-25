---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    discount,
    browser
FROM smelt.ref('py_l2_282')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_108') WHERE quantity > 0
)
