---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT segment, score, campaign_id, 'source_0' AS source_tag FROM smelt.sql_l2_225
UNION ALL
SELECT segment, score, campaign_id, 'source_1' AS source_tag FROM smelt.sql_l2_27
UNION ALL
SELECT segment, score, campaign_id, 'source_2' AS source_tag FROM smelt.sql_l2_101
