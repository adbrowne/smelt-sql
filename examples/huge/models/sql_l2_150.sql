---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.event_date,
    a.region,
    b.browser
FROM smelt.ref('sql_l1_173') a
INNER JOIN smelt.ref('sql_l1_64') b ON a.user_id = b.user_id
