---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT plan_type, created_at, transaction_id, 'source_0' AS source_tag FROM smelt.models.sql_l1_32
UNION ALL
SELECT plan_type, created_at, transaction_id, 'source_1' AS source_tag FROM smelt.models.sql_l1_153
UNION ALL
SELECT plan_type, created_at, transaction_id, 'source_2' AS source_tag FROM smelt.models.sql_l1_10

