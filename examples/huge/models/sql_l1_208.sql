---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.quantity,
    a.product_id,
    b.cost
FROM smelt.models.categories a
INNER JOIN smelt.models.categories b ON a.user_id = b.user_id

