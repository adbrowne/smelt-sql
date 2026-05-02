---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT browser, transaction_id, event_type, 'source_0' AS source_tag FROM smelt.models.sql_l1_117
UNION ALL
SELECT browser, transaction_id, event_type, 'source_1' AS source_tag FROM smelt.models.sql_l1_50
UNION ALL
SELECT browser, transaction_id, event_type, 'source_2' AS source_tag FROM smelt.models.sql_l1_156

