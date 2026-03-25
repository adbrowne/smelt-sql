---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT score, status, price, 'source_0' AS source_tag FROM smelt.ref('sql_l1_110')
UNION ALL
SELECT score, status, price, 'source_1' AS source_tag FROM smelt.ref('sql_l1_44')
UNION ALL
SELECT score, status, price, 'source_2' AS source_tag FROM smelt.ref('py_l1_281')
