---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    segment,
    is_verified,
    referrer
FROM smelt.ref('py_l3_490')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_490') WHERE score >= 50
)
