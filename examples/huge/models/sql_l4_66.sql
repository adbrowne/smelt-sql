---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    segment,
    session_id,
    page_path,
    cost
FROM smelt.ref('py_l3_494')
WHERE status = 'active'
