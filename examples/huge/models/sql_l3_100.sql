---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT browser, session_id, quantity, 'source_0' AS source_tag FROM smelt.ref('sql_l2_58')
UNION ALL
SELECT browser, session_id, quantity, 'source_1' AS source_tag FROM smelt.ref('py_l2_418')
UNION ALL
SELECT browser, session_id, quantity, 'source_2' AS source_tag FROM smelt.ref('py_l2_318')
