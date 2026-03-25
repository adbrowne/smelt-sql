---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT ip_address, is_active, email_domain, 'source_0' AS source_tag FROM smelt.ref('page_views')
UNION ALL
SELECT ip_address, is_active, email_domain, 'source_1' AS source_tag FROM smelt.ref('page_views')
