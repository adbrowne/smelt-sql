---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT os_name, price, is_verified, 'source_0' AS source_tag FROM smelt.ref('sql_l2_162')
UNION ALL
SELECT os_name, price, is_verified, 'source_1' AS source_tag FROM smelt.ref('py_l2_389')
UNION ALL
SELECT os_name, price, is_verified, 'source_2' AS source_tag FROM smelt.ref('py_l2_384')
