---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    category,
    browser
FROM smelt.ref('py_l1_340')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_340') WHERE category IS NOT NULL
)
