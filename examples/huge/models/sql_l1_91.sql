---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.platform,
    a.created_at,
    b.amount
FROM smelt.ref('signups') a
INNER JOIN smelt.ref('signups') b ON a.user_id = b.user_id
