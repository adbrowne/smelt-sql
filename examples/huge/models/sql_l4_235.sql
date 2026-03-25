---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    a.page_path,
    b.price
FROM smelt.ref('sql_l3_149') a
LEFT JOIN smelt.ref('sql_l3_184') b ON a.user_id = b.user_id
