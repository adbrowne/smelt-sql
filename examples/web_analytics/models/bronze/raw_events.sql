-- Bronze passthrough — provides a named model for downstream silver
-- transformations to attach to instead of binding directly to the raw source.
SELECT
    event_id,
    device_id,
    user_id,
    seconds_in_day,
    payload,
    event_date
FROM smelt.sources.raw.events
