---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT ip_address, is_active, email_domain, 'source_0' AS source_tag FROM smelt.models.page_views
UNION ALL
SELECT ip_address, is_active, email_domain, 'source_1' AS source_tag FROM smelt.models.page_views

