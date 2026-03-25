---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    cohort_date,
    created_at
FROM smelt.ref('py_l2_313')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_11') WHERE quantity > 0
)
