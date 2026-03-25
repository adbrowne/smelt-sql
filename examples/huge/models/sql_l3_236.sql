---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    duration_seconds,
    LAG(amount, 1) OVER (PARTITION BY campaign_id ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_130')
