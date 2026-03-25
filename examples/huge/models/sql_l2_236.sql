---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT category, event_type, platform, 'source_0' AS source_tag FROM smelt.ref('sql_l1_82')
UNION ALL
SELECT category, event_type, platform, 'source_1' AS source_tag FROM smelt.ref('py_l1_419')
