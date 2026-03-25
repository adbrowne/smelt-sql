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
    b.region,
    c.discount,
    c.plan_type
FROM smelt.ref('sql_l1_192') a
INNER JOIN smelt.ref('sql_l1_92') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l1_192') c ON a.user_id = c.user_id
