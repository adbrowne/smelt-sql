---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT plan_type, quantity, price, 'source_0' AS source_tag FROM smelt.models.sql_l2_170
UNION ALL
SELECT plan_type, quantity, price, 'source_1' AS source_tag FROM smelt.models.sql_l2_166

