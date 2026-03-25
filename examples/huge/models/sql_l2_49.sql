---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    page_path,
    event_type,
    platform
FROM smelt.ref('py_l1_355')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_260') WHERE category IS NOT NULL
)
