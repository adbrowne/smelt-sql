-- User sessions derived from events
SELECT
    user_id,
    session_id,
    COUNT(*) as event_count
FROM sqt.ref('raw_events', filter => event_type = 'page_view')
GROUP BY user_id, session_id
