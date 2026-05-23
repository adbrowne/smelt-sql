---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.browser,
    b.order_id,
    c.duration_seconds,
    c.segment
FROM smelt.invoices a
INNER JOIN smelt.invoices b ON a.user_id = b.user_id
LEFT JOIN smelt.invoices c ON a.user_id = c.user_id
