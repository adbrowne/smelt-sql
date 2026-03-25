---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.email_domain,
    b.device_type,
    c.user_id,
    c.is_active
FROM smelt.ref('sql_l1_236') a
INNER JOIN smelt.ref('sql_l1_162') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_26') c ON a.user_id = c.user_id
