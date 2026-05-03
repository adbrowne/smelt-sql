---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT revenue, country, ip_address, 'source_0' AS source_tag FROM smelt.errors
UNION ALL
SELECT revenue, country, ip_address, 'source_1' AS source_tag FROM smelt.errors

