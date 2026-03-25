---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    email_domain,
    LAG(amount, 1) OVER (PARTITION BY campaign_id ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_275')
