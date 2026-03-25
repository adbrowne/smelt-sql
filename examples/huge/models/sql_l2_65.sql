---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.campaign_id,
    a.order_id,
    b.cohort_date
FROM smelt.ref('py_l1_420') a
INNER JOIN smelt.ref('sql_l1_247') b ON a.user_id = b.user_id
