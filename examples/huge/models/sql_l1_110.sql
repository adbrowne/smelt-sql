---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_active,
    a.platform,
    b.browser
FROM smelt.ref('products') a
INNER JOIN smelt.ref('products') b ON a.user_id = b.user_id
