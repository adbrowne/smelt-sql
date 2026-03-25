---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT product_id, session_id, profit, 'source_0' AS source_tag FROM smelt.ref('py_l1_481')
UNION ALL
SELECT product_id, session_id, profit, 'source_1' AS source_tag FROM smelt.ref('py_l1_302')
