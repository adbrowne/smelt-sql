---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.created_at,
    a.campaign_id,
    b.referrer
FROM smelt.ref('sql_l1_11') a
INNER JOIN smelt.ref('py_l1_292') b ON a.user_id = b.user_id
