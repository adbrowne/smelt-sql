---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.tier,
    a.revenue,
    b.channel
FROM smelt.ref('subscriptions') a
INNER JOIN smelt.ref('subscriptions') b ON a.user_id = b.user_id
