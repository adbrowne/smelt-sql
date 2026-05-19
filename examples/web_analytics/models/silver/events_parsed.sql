-- Compose event_ts from the partition date + sub-day offset, and project the
-- JSON payload fields as typed columns via parse_event_payload.
SELECT
    event_id,
    device_id,
    user_id,
    CAST(event_date AS DATE) + to_seconds(seconds_in_day) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    smelt.functions.parse_event_payload(payload).*
FROM smelt.bronze.raw_events
