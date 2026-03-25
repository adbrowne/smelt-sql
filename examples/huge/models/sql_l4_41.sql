---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    category,
    browser,
    segment,
    updated_at
FROM smelt.ref('py_l3_390')
WHERE score >= 50
