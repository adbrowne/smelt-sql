---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    plan_type,
    CASE
        WHEN amount > 1000 THEN 'high'
        WHEN amount > 100 THEN 'medium'
        ELSE 'low'
    END AS value_tier
FROM smelt.ref('invoices')
