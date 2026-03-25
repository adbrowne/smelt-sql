---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT session_id, category, segment, 'source_0' AS source_tag FROM smelt.ref('sql_l3_189')
UNION ALL
SELECT session_id, category, segment, 'source_1' AS source_tag FROM smelt.ref('py_l3_398')
UNION ALL
SELECT session_id, category, segment, 'source_2' AS source_tag FROM smelt.ref('sql_l3_199')
