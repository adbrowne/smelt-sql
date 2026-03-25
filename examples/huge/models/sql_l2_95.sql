---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT score, category, campaign_id, 'source_0' AS source_tag FROM smelt.ref('py_l1_276')
UNION ALL
SELECT score, category, campaign_id, 'source_1' AS source_tag FROM smelt.ref('py_l1_325')
