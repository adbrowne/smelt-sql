---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT segment, amount, region, 'source_0' AS source_tag FROM smelt.models.sql_l2_206
UNION ALL
SELECT segment, amount, region, 'source_1' AS source_tag FROM smelt.models.sql_l2_163

