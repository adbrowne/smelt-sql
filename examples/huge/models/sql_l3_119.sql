---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT segment, score, campaign_id, 'source_0' AS source_tag FROM smelt.ref('py_l2_382')
UNION ALL
SELECT segment, score, campaign_id, 'source_1' AS source_tag FROM smelt.ref('py_l2_308')
