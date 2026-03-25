---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT region, is_verified, browser, 'source_0' AS source_tag FROM smelt.ref('sql_l3_55')
UNION ALL
SELECT region, is_verified, browser, 'source_1' AS source_tag FROM smelt.ref('sql_l3_55')
