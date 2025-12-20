-- Raw events from source system
SELECT
    event_id,
    user_id,
    event_time,
    event_type
FROM source.events
