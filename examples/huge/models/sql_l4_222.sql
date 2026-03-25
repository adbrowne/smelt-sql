---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT region, is_verified, browser, 'source_0' AS source_tag FROM smelt.ref('sql_l3_155')
UNION ALL
SELECT region, is_verified, browser, 'source_1' AS source_tag FROM smelt.ref('py_l3_435')
UNION ALL
SELECT region, is_verified, browser, 'source_2' AS source_tag FROM smelt.ref('py_l3_281')
