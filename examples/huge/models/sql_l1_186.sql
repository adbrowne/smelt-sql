---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.score,
    a.quantity,
    b.discount
FROM smelt.ref('payments') a
INNER JOIN smelt.ref('payments') b ON a.user_id = b.user_id
