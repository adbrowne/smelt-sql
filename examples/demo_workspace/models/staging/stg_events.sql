-- Cleaned event stream with revenue in dollars
SELECT
    event_id,
    user_id,
    event_type,
    event_time,
    revenue_cents
FROM smelt.sources.raw.events

