-- User statistics
SELECT
    user_id,
    COUNT(*) as total_sessions,
    SUM(event_count) as total_events
FROM smelt.ref('user_sessions')
GROUP BY user_id
