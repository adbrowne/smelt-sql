---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT updated_at, country, plan_type, 'source_0' AS source_tag FROM smelt.models.events
UNION ALL
SELECT updated_at, country, plan_type, 'source_1' AS source_tag FROM smelt.models.events

