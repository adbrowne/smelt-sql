-- User sessions derived from events
SELECT
    user_id,
    DATE_TRUNC('day', event_time) as session_id,
    COUNT(*) as event_count
FROM smelt.ref('raw_events')
WHERE event_type = 'page_view'
GROUP BY user_id, session_id
