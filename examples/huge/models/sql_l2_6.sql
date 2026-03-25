---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    duration_seconds,
    amount
FROM smelt.ref('py_l1_445')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_407') WHERE score >= 50
)
