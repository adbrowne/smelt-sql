---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.tier,
    b.score,
    c.profit,
    c.campaign_id
FROM smelt.sql_l3_102 a
INNER JOIN smelt.sql_l3_102 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_102 c ON a.user_id = c.user_id

