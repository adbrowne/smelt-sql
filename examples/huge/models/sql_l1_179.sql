---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.region,
    b.event_time,
    c.tier,
    c.discount
FROM smelt.models.invoices a
INNER JOIN smelt.models.invoices b ON a.user_id = b.user_id
LEFT JOIN smelt.models.invoices c ON a.user_id = c.user_id

