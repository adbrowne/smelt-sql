---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT tier, campaign_id, profit, 'source_0' AS source_tag FROM smelt.sql_l3_192
UNION ALL
SELECT tier, campaign_id, profit, 'source_1' AS source_tag FROM smelt.sql_l3_83

