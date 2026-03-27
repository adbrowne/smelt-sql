--- name: test_user_activity ---
materialization: test
test:
  model: user_activity
  inputs:
    users:
      - {user_id: 1, user_name: Alice, signup_date: '2024-01-01'}
      - {user_id: 2, user_name: Bob, signup_date: '2024-02-01'}
    events:
      - {event_id: 1, user_id: 1, event_type: page_view, event_timestamp: '2024-01-15 10:00:00', properties: null}
      - {event_id: 2, user_id: 1, event_type: click, event_timestamp: '2024-01-16 11:00:00', properties: null}
      - {event_id: 3, user_id: 2, event_type: page_view, event_timestamp: '2024-02-15 09:00:00', properties: null}
  expect:
    - {user_id: 1, user_name: Alice, total_events: 2}
    - {user_id: 2, user_name: Bob, total_events: 1}
---
