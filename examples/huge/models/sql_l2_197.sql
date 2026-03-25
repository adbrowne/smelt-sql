---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT rating, country, profit, 'source_0' AS source_tag FROM smelt.ref('py_l1_408')
UNION ALL
SELECT rating, country, profit, 'source_1' AS source_tag FROM smelt.ref('py_l1_408')
