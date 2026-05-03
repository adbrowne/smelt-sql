---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.session_id,
    b.os_name,
    c.product_id,
    c.page_path
FROM smelt.logs a
INNER JOIN smelt.logs b ON a.user_id = b.user_id
LEFT JOIN smelt.logs c ON a.user_id = c.user_id

