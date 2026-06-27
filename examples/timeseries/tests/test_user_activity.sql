-- Unit test for the user_activity model.
-- Deps smelt.users and smelt.events are single-segment so PASSING names
-- map directly to the model's external dep keys.

smelt.test test_user_activity AS (
    SELECT
        u.user_id,
        u.user_name,
        COUNT(e.event_id) AS total_events
    FROM smelt.users u
    INNER JOIN smelt.events e ON u.user_id = e.user_id
    GROUP BY u.user_id, u.user_name
)
PASSING users AS (
    {user_id: 1, user_name: 'Alice', signup_date: '2024-01-01'},
    {user_id: 2, user_name: 'Bob', signup_date: '2024-02-01'}
)
PASSING events AS (
    {event_id: 1, user_id: 1, event_type: 'page_view', event_timestamp: '2024-01-15 10:00:00', properties: null},
    {event_id: 2, user_id: 1, event_type: 'click',     event_timestamp: '2024-01-16 11:00:00', properties: null},
    {event_id: 3, user_id: 2, event_type: 'page_view', event_timestamp: '2024-02-15 09:00:00', properties: null}
)
EXPECT (
    {user_id: 1, user_name: 'Alice', total_events: 2},
    {user_id: 2, user_name: 'Bob',   total_events: 1}
)
