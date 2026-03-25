---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT price, segment, os_name, 'source_0' AS source_tag FROM smelt.ref('py_l2_330')
UNION ALL
SELECT price, segment, os_name, 'source_1' AS source_tag FROM smelt.ref('py_l2_486')
