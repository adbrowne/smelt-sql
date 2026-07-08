---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT ip_address, is_active, email_domain, 'source_0' AS source_tag FROM smelt.page_views
UNION ALL
SELECT ip_address, is_active, email_domain, 'source_1' AS source_tag FROM smelt.page_views
