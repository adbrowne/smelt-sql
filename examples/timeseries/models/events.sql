-- Event data from raw source
SELECT
    event_id,
    user_id,
    event_type,
    event_timestamp,
    properties
FROM smelt.sources.raw.events

