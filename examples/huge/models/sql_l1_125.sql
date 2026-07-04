---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
WITH base AS (
    SELECT created_at, product_id, device_type
    FROM smelt.invoices
    WHERE category IS NOT NULL
)
SELECT
    b.created_at,
    MAX(created_at) AS agg_val
FROM base b
INNER JOIN smelt.invoices j ON b.user_id = j.user_id
GROUP BY b.created_at
