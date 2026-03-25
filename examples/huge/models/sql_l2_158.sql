---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT segment, rating, campaign_id, 'source_0' AS source_tag FROM smelt.ref('sql_l1_98')
UNION ALL
SELECT segment, rating, campaign_id, 'source_1' AS source_tag FROM smelt.ref('sql_l1_118')
