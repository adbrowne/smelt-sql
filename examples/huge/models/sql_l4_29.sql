---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.referrer,
    a.channel,
    b.status
FROM smelt.ref('py_l3_437') a
INNER JOIN smelt.ref('sql_l3_201') b ON a.user_id = b.user_id
