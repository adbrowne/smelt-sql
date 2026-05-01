---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.user_id,
    a.campaign_id,
    b.duration_seconds
FROM smelt.models.sql_l1_32 a
INNER JOIN smelt.models.sql_l1_54 b ON a.user_id = b.user_id

