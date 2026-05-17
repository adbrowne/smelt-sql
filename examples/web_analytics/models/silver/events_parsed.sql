-- Compose event_ts from the partition date + sub-day offset, and extract the
-- JSON payload fields into typed columns. Downstream models reference
-- event_name, platform, and url directly instead of re-parsing the JSON every time.
SELECT
    event_id,
    device_id,
    user_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    json_extract_string(payload, '$.event_name') AS event_name,
    json_extract_string(payload, '$.platform') AS platform,
    json_extract_string(payload, '$.url') AS url
FROM smelt.bronze.raw_events
