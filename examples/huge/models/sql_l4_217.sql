---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT is_active, channel, platform, 'source_0' AS source_tag FROM smelt.ref('sql_l3_183')
UNION ALL
SELECT is_active, channel, platform, 'source_1' AS source_tag FROM smelt.ref('sql_l3_183')
