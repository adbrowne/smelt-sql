-- Cube split optimization: multiple COUNT(DISTINCT) split into parallel sub-queries
-- Run with: smelt run --select cube_metrics
SELECT
    date_trunc('day', event_timestamp) as event_date,
    event_type,
    COUNT(DISTINCT user_id) as unique_users,
    COUNT(DISTINCT event_id) as unique_events,
    COUNT(*) as total_events
FROM smelt.sources.raw.events
GROUP BY 1, 2 -- smelt:cube_split

