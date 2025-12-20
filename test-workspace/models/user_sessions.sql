-- User sessions derived from events
SELECT
    user_id,
    session_id,
    COUNT(*) as event_count
FROM ref('raw_events')
GROUP BY user_id, session_id
