---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.is_verified,
    a.plan_type,
    b.campaign_id
FROM smelt.ref('sql_l2_132') a
INNER JOIN smelt.ref('sql_l2_59') b ON a.user_id = b.user_id
