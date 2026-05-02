---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT plan_type, updated_at, browser, 'source_0' AS source_tag FROM smelt.models.sql_l3_234
UNION ALL
SELECT plan_type, updated_at, browser, 'source_1' AS source_tag FROM smelt.models.sql_l3_170
UNION ALL
SELECT plan_type, updated_at, browser, 'source_2' AS source_tag FROM smelt.models.sql_l3_190

