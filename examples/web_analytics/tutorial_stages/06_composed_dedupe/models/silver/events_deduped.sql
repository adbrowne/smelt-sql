---
materialization: table
refresh: incremental
unique_key: [event_id]
grain: key
timeseries:
  event_time_column: first_seen_date
  partition_column: first_seen_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      raw.events:
        allow_full_scan: true
---
-- The composed shape — key-addressed (one row per `event_id`, via
-- `unique_key:`, which derives `grain: key`) *and* time-partitioned
-- (`first_seen_date`, via `timeseries:`). Locality is
-- established by route 3 (recurrence-bounded): `smelt.sources.raw.events`
-- declares `mutation_profile.key_recurrence`, and every duplicate delivery
-- of one `event_id` is bounded by that window on the event-time axis.
-- Redelivery dedup falls out of the keyed merge itself — every column
-- folds through `MIN`, which converges to the same value regardless of
-- which copy of a redelivered event a run happens to see — so this model
-- needs neither the QUALIFY window nor the safety override the previous
-- pages required.
SELECT
    event_id,
    MIN(device_id) AS device_id,
    MIN(user_id) AS user_id,
    MIN(CASE WHEN user_id IS NOT NULL
             THEN 'u:' || CAST(user_id AS VARCHAR)
             ELSE 'd:' || CAST(device_id AS VARCHAR)
        END) AS amplitude_id,
    MIN(CAST(event_time AS TIMESTAMP)) AS event_ts,
    MIN(CAST(event_date AS DATE)) AS first_seen_date,
    MIN(utm_campaign) AS utm_campaign,
    MIN(json_extract_string(payload, '$.event_name')) AS event_name,
    MIN(json_extract_string(payload, '$.platform')) AS platform,
    MIN(json_extract_string(payload, '$.url')) AS url
FROM smelt.sources.raw.events
GROUP BY event_id
