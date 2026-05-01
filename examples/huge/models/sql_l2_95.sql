---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT score, category, campaign_id, 'source_0' AS source_tag FROM smelt.models.sql_l1_19
UNION ALL
SELECT score, category, campaign_id, 'source_1' AS source_tag FROM smelt.models.sql_l1_126
UNION ALL
SELECT score, category, campaign_id, 'source_2' AS source_tag FROM smelt.models.sql_l1_12

