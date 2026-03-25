---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT segment, amount, region, 'source_0' AS source_tag FROM smelt.ref('py_l2_388')
UNION ALL
SELECT segment, amount, region, 'source_1' AS source_tag FROM smelt.ref('py_l2_379')
UNION ALL
SELECT segment, amount, region, 'source_2' AS source_tag FROM smelt.ref('sql_l2_126')
