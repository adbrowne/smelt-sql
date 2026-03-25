---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT profit, ip_address, event_date, 'source_0' AS source_tag FROM smelt.ref('sql_l3_127')
UNION ALL
SELECT profit, ip_address, event_date, 'source_1' AS source_tag FROM smelt.ref('sql_l3_127')
