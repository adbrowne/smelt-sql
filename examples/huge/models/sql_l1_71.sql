---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.updated_at,
    b.is_verified,
    c.campaign_id,
    c.plan_type
FROM smelt.ref('reviews') a
INNER JOIN smelt.ref('reviews') b ON a.user_id = b.user_id
LEFT JOIN smelt.ref('reviews') c ON a.user_id = c.user_id
