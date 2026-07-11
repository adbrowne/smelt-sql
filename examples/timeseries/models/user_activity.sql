-- User activity summary
-- Uses DuckDB idioms: TRY_CAST (error-tolerant cast, always nullable) and
-- GROUP BY ALL (group by every non-aggregate select item).
SELECT
    u.user_id,
    u.user_name,
    u.signup_date,
    TRY_CAST(u.user_name AS BIGINT) as user_name_as_number,
    COUNT(e.event_id) as total_events
FROM smelt.users u
INNER JOIN smelt.events e ON u.user_id = e.user_id
GROUP BY ALL

