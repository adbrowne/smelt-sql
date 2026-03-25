---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    category,
    browser
FROM smelt.ref('sql_l3_151')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_87') WHERE is_active = true
)
