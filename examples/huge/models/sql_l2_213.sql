---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT price, campaign_id, rating, 'source_0' AS source_tag FROM smelt.ref('sql_l1_170')
UNION ALL
SELECT price, campaign_id, rating, 'source_1' AS source_tag FROM smelt.ref('sql_l1_170')
