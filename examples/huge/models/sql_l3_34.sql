---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT event_type, transaction_id, updated_at, 'source_0' AS source_tag FROM smelt.sql_l2_115
UNION ALL
SELECT event_type, transaction_id, updated_at, 'source_1' AS source_tag FROM smelt.sql_l2_43
UNION ALL
SELECT event_type, transaction_id, updated_at, 'source_2' AS source_tag FROM smelt.sql_l2_175

