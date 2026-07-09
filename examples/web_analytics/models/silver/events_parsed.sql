---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Compose event_ts from the partition date + sub-day offset, project the JSON
-- payload fields as typed columns via parse_event_payload, and synthesise the
-- amplitude_id — the Amplitude-style never-NULL identifier that prefers the
-- signed-in user_id and falls back to the device_id. The 'u:' / 'd:' prefix
-- keeps the user-id and device-id namespaces disjoint so cross-device
-- collisions are impossible. amplitude_id is the no-merging baseline that the
-- three gold-layer identity algorithms refine into more aggressive partitions.
SELECT
    event_id,
    device_id,
    user_id,
    CASE WHEN user_id IS NOT NULL
         THEN 'u:' || CAST(user_id AS VARCHAR)
         ELSE 'd:' || CAST(device_id AS VARCHAR)
    END AS amplitude_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    smelt.functions.parse_event_payload(payload).*
FROM smelt.bronze.raw_events
