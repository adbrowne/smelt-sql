---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT email_domain, platform, order_id, 'source_0' AS source_tag FROM smelt.ref('users')
UNION ALL
SELECT email_domain, platform, order_id, 'source_1' AS source_tag FROM smelt.ref('users')
