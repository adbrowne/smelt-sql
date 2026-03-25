---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.product_id,
    a.os_name,
    b.discount
FROM smelt.ref('clicks') a
INNER JOIN smelt.ref('clicks') b ON a.user_id = b.user_id
