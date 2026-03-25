---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.platform,
    b.is_verified,
    c.is_active,
    c.plan_type
FROM smelt.ref('errors') a
INNER JOIN smelt.ref('errors') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('errors') c ON a.user_id = c.user_id
