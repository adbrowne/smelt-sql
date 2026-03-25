---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.os_name,
    a.campaign_id,
    b.rating
FROM smelt.ref('sql_l2_199') a
LEFT JOIN smelt.ref('sql_l2_14') b ON a.user_id = b.user_id
