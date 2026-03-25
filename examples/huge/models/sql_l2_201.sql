---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    ip_address,
    cohort_date,
    transaction_id,
    referrer
FROM smelt.ref('sql_l1_85')
WHERE created_at >= '2024-01-01'
