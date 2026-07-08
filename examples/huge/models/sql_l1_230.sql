---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT email_domain, platform, order_id, 'source_0' AS source_tag FROM smelt.users
UNION ALL
SELECT email_domain, platform, order_id, 'source_1' AS source_tag FROM smelt.users
