---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT referrer, event_date, plan_type, 'source_0' AS source_tag FROM smelt.models.reviews
UNION ALL
SELECT referrer, event_date, plan_type, 'source_1' AS source_tag FROM smelt.models.reviews

