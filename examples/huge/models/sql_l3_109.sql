---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT amount, category, rating, 'source_0' AS source_tag FROM smelt.ref('sql_l2_199')
UNION ALL
SELECT amount, category, rating, 'source_1' AS source_tag FROM smelt.ref('sql_l2_111')
UNION ALL
SELECT amount, category, rating, 'source_2' AS source_tag FROM smelt.ref('py_l2_262')
