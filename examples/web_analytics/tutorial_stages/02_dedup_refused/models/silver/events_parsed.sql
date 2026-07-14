---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
-- Tutorial stage 2 (see docs-site examples/web-analytics): stage 1 plus
-- redelivery dedup — but WITHOUT the safety override the dedup window
-- needs. This stage exists to capture smelt's real refusal: the QUALIFY
-- window partitions by event_id, not the model's partition column, and
-- smelt cannot prove it is safe to run partition-by-partition. `smelt run
-- --dry-run` on this workspace exits non-zero with the refusal message.
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
QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
