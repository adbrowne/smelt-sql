---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.user_id,
    a.campaign_id,
    b.duration_seconds
FROM smelt.ref('sql_l1_232') a
INNER JOIN smelt.ref('sql_l1_232') b ON a.user_id = b.user_id
