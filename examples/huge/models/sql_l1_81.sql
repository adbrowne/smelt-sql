---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT is_verified, region, event_type, 'source_0' AS source_tag FROM smelt.models.clicks
UNION ALL
SELECT is_verified, region, event_type, 'source_1' AS source_tag FROM smelt.models.clicks

