---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT updated_at, transaction_id, revenue, 'source_0' AS source_tag FROM smelt.sql_l3_117
UNION ALL
SELECT updated_at, transaction_id, revenue, 'source_1' AS source_tag FROM smelt.sql_l3_158
UNION ALL
SELECT updated_at, transaction_id, revenue, 'source_2' AS source_tag FROM smelt.sql_l3_208

