---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Tutorial stage 1 (see docs-site examples/web-analytics): the simplest
-- version of silver.events_parsed. Types the raw feed, projects the JSON
-- payload fields as columns, and synthesises the amplitude_id. No
-- deduplication, no late-arrival handling yet — those arrive in stage 2.
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
