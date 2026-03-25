---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT os_name, amount, segment, 'source_0' AS source_tag FROM smelt.ref('sql_l1_157')
UNION ALL
SELECT os_name, amount, segment, 'source_1' AS source_tag FROM smelt.ref('py_l1_321')
UNION ALL
SELECT os_name, amount, segment, 'source_2' AS source_tag FROM smelt.ref('sql_l1_237')
