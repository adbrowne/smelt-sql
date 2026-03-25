---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    referrer,
    device_type,
    score
FROM smelt.ref('py_l1_271')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_467') WHERE amount > 0
)
