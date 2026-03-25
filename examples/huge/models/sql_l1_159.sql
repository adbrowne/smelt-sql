---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.region,
    a.campaign_id,
    b.event_time
FROM smelt.ref('clicks') a
INNER JOIN smelt.ref('clicks') b ON a.user_id = b.user_id
