---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    ip_address,
    country
FROM smelt.ref('sql_l3_227')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_177') WHERE country = 'US'
)
