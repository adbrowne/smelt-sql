---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    status,
    revenue,
    campaign_id
FROM smelt.ref('sql_l3_214')
WHERE amount > 0
