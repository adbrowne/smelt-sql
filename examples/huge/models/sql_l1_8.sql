---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    browser,
    updated_at,
    quantity
FROM smelt.ref('errors')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('errors') WHERE is_active = true
)
