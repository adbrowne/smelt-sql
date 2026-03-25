---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT transaction_id, event_date, campaign_id, 'source_0' AS source_tag FROM smelt.ref('py_l1_474')
UNION ALL
SELECT transaction_id, event_date, campaign_id, 'source_1' AS source_tag FROM smelt.ref('py_l1_474')
