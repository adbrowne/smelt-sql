---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT user_id, amount, channel, 'source_0' AS source_tag FROM smelt.models.sql_l1_10
UNION ALL
SELECT user_id, amount, channel, 'source_1' AS source_tag FROM smelt.models.sql_l1_10

