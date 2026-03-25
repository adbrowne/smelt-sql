---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    campaign_id,
    LAG(amount, 1) OVER (PARTITION BY duration_seconds ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_471')
