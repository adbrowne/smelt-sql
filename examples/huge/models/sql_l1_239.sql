---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    platform,
    is_verified
FROM smelt.ref('page_views')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('page_views') WHERE score >= 50
)
