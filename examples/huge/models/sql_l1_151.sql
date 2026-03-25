---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_type,
    a.profit,
    b.is_verified
FROM smelt.ref('page_views') a
INNER JOIN smelt.ref('page_views') b ON a.user_id = b.user_id
