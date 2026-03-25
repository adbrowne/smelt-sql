---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT product_id, platform, duration_seconds, 'source_0' AS source_tag FROM smelt.ref('sql_l3_63')
UNION ALL
SELECT product_id, platform, duration_seconds, 'source_1' AS source_tag FROM smelt.ref('sql_l3_63')
