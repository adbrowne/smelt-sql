---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT profit, revenue, user_id, 'source_0' AS source_tag FROM smelt.ref('sql_l1_92')
UNION ALL
SELECT profit, revenue, user_id, 'source_1' AS source_tag FROM smelt.ref('sql_l1_92')
