---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT rating, user_id, price, 'source_0' AS source_tag FROM smelt.ref('users')
UNION ALL
SELECT rating, user_id, price, 'source_1' AS source_tag FROM smelt.ref('users')
