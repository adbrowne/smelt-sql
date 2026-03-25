---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT rating, score, amount, 'source_0' AS source_tag FROM smelt.ref('sql_l1_141')
UNION ALL
SELECT rating, score, amount, 'source_1' AS source_tag FROM smelt.ref('sql_l1_63')
UNION ALL
SELECT rating, score, amount, 'source_2' AS source_tag FROM smelt.ref('py_l1_330')
