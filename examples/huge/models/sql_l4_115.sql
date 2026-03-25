---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT channel, plan_type, event_time, 'source_0' AS source_tag FROM smelt.ref('sql_l3_99')
UNION ALL
SELECT channel, plan_type, event_time, 'source_1' AS source_tag FROM smelt.ref('sql_l3_93')
UNION ALL
SELECT channel, plan_type, event_time, 'source_2' AS source_tag FROM smelt.ref('py_l3_466')
