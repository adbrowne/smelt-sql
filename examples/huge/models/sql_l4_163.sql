---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT browser, device_type, is_verified, 'source_0' AS source_tag FROM smelt.ref('py_l3_350')
UNION ALL
SELECT browser, device_type, is_verified, 'source_1' AS source_tag FROM smelt.ref('py_l3_409')
UNION ALL
SELECT browser, device_type, is_verified, 'source_2' AS source_tag FROM smelt.ref('py_l3_281')
