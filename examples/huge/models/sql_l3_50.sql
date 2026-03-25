---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.channel,
    a.event_date,
    b.user_id
FROM smelt.ref('sql_l2_51') a
LEFT JOIN smelt.ref('sql_l2_212') b ON a.user_id = b.user_id
