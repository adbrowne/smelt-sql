-- User activity summary
SELECT
    u.user_id,
    u.user_name,
    u.signup_date,
    COUNT(e.event_id) as total_events
FROM smelt.users u
INNER JOIN smelt.events e ON u.user_id = e.user_id
GROUP BY u.user_id, u.user_name, u.signup_date

