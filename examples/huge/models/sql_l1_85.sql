---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.channel,
    a.campaign_id,
    b.cost
FROM smelt.ref('reviews') a
LEFT JOIN smelt.ref('reviews') b ON a.user_id = b.user_id
