---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.tier,
    b.score,
    c.profit,
    c.campaign_id
FROM smelt.ref('sql_l3_38') a
INNER JOIN smelt.ref('sql_l3_38') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('sql_l3_38') c ON a.user_id = c.user_id
