---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT revenue, country, ip_address, 'source_0' AS source_tag FROM smelt.ref('errors')
UNION ALL
SELECT revenue, country, ip_address, 'source_1' AS source_tag FROM smelt.ref('errors')
