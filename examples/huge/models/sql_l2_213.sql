---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT price, campaign_id, rating, 'source_0' AS source_tag FROM smelt.ref('sql_l1_128')
UNION ALL
SELECT price, campaign_id, rating, 'source_1' AS source_tag FROM smelt.ref('sql_l1_130')
UNION ALL
SELECT price, campaign_id, rating, 'source_2' AS source_tag FROM smelt.ref('sql_l1_98')
