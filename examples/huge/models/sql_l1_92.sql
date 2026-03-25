---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.is_verified,
    a.browser,
    b.discount
FROM smelt.ref('errors') a
LEFT JOIN smelt.ref('errors') b ON a.user_id = b.user_id
