---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_type,
    a.is_verified,
    b.ip_address
FROM smelt.ref('sql_l1_220') a
LEFT JOIN smelt.ref('sql_l1_104') b ON a.user_id = b.user_id
