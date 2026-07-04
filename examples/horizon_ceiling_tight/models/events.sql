---
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
---
SELECT
    event_id,
    user_id,
    event_ts,
    date_trunc('day', event_ts)::DATE AS event_date,
    amount
FROM smelt.sources.raw_events
