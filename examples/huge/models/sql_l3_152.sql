---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.channel,
    a.amount,
    b.browser
FROM smelt.ref('sql_l2_76') a
INNER JOIN smelt.ref('sql_l2_43') b ON a.user_id = b.user_id
