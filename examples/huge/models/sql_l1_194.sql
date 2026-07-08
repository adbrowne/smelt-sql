---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT duration_seconds, cost, score, 'source_0' AS source_tag FROM smelt.campaigns
UNION ALL
SELECT duration_seconds, cost, score, 'source_1' AS source_tag FROM smelt.campaigns
