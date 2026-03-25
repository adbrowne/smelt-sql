---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT browser, rating, updated_at, 'source_0' AS source_tag FROM smelt.ref('sql_l2_106')
UNION ALL
SELECT browser, rating, updated_at, 'source_1' AS source_tag FROM smelt.ref('sql_l2_32')
UNION ALL
SELECT browser, rating, updated_at, 'source_2' AS source_tag FROM smelt.ref('sql_l2_161')
