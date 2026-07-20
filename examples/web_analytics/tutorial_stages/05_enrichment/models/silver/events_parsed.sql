---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
# The redelivery dedup window partitions by event_id, not event_date — the
# analyzer cannot statically prove it is partition-aligned. It is safe in
# practice: a redelivered duplicate always lands in the *same* event_date
# partition as its original, so the window never needs to see across a
# partition boundary to resolve one event_id's duplicates.
safety_overrides:
  allow_window_functions: true
---
-- Tutorial stage 3 (see docs-site examples/web-analytics): stage 1 plus
-- both upstream-hygiene rules of the full example's silver.events_parsed —
-- redelivery dedup (QUALIFY keeps the earliest-arriving copy per event_id)
-- and the 3-day late-arrival acceptance window (the WHERE filter below,
-- which smelt reads as a genuine 3-day lookback on bronze.raw_events).
-- Identical to the canonical model except the payload parsing is inlined
-- here instead of factored into functions/parse_event_payload.sql.
SELECT
    event_id,
    device_id,
    user_id,
    CASE WHEN user_id IS NOT NULL
         THEN 'u:' || CAST(user_id AS VARCHAR)
         ELSE 'd:' || CAST(device_id AS VARCHAR)
    END AS amplitude_id,
    CAST(event_time AS TIMESTAMP) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    utm_campaign,
    json_extract_string(payload, '$.event_name') AS event_name,
    json_extract_string(payload, '$.platform') AS platform,
    json_extract_string(payload, '$.url') AS url
FROM smelt.bronze.raw_events
WHERE event_date
    BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
        AND CAST(arrival_time AS DATE)
QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
