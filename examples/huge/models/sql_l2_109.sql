---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT segment, updated_at, is_verified, 'source_0' AS source_tag FROM smelt.ref('sql_l1_135')
UNION ALL
SELECT segment, updated_at, is_verified, 'source_1' AS source_tag FROM smelt.ref('sql_l1_135')
