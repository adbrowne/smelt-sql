---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT os_name, revenue, event_time, 'source_0' AS source_tag FROM smelt.ref('sql_l1_127')
UNION ALL
SELECT os_name, revenue, event_time, 'source_1' AS source_tag FROM smelt.ref('py_l1_324')
UNION ALL
SELECT os_name, revenue, event_time, 'source_2' AS source_tag FROM smelt.ref('sql_l1_90')
