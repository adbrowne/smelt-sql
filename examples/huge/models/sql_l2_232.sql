---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.order_id,
    a.page_path,
    b.campaign_id
FROM smelt.ref('sql_l1_177') a
INNER JOIN smelt.ref('sql_l1_20') b ON a.user_id = b.user_id
