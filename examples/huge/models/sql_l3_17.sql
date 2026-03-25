---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    a.plan_type,
    b.campaign_id
FROM smelt.ref('sql_l2_70') a
INNER JOIN smelt.ref('py_l2_413') b ON a.user_id = b.user_id
