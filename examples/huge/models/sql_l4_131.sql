---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    duration_seconds,
    channel
FROM smelt.ref('py_l3_386')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_386') WHERE category IS NOT NULL
)
