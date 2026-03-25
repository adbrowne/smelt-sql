---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT tier, campaign_id, profit, 'source_0' AS source_tag FROM smelt.ref('sql_l3_4')
UNION ALL
SELECT tier, campaign_id, profit, 'source_1' AS source_tag FROM smelt.ref('sql_l3_8')
