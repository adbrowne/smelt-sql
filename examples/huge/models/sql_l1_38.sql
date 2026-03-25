---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.discount,
    a.plan_type,
    b.category
FROM smelt.ref('campaigns') a
INNER JOIN smelt.ref('campaigns') b ON a.user_id = b.user_id
