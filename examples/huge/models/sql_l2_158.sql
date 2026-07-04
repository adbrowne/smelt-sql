---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT segment, rating, campaign_id, 'source_0' AS source_tag FROM smelt.sql_l1_208
UNION ALL
SELECT segment, rating, campaign_id, 'source_1' AS source_tag FROM smelt.sql_l1_44
UNION ALL
SELECT segment, rating, campaign_id, 'source_2' AS source_tag FROM smelt.sql_l1_65
