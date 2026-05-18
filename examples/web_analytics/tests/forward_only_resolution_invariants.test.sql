--- name: test_forward_only_resolution_invariants ---
materialization: test
test:
  model: identity_forward_only
  inputs:
    silver_sessions:
      # Session A: one signed-in event at the end (event id 2 carries user_id 100)
      - {session_id: 'sa', device_id: 1, session_seq: 0, session_start: '2026-04-01 10:00:00', session_end: '2026-04-01 10:10:00', session_start_date: '2026-04-01', event_count: 2, platform: 'web'}
      # Session B: two signed-in events; the LATER one (event id 5 at 11:08, user_id 201) wins
      - {session_id: 'sb', device_id: 2, session_seq: 0, session_start: '2026-04-01 11:00:00', session_end: '2026-04-01 11:10:00', session_start_date: '2026-04-01', event_count: 3, platform: 'web'}
      # Session C: zero signed-in events
      - {session_id: 'sc', device_id: 3, session_seq: 0, session_start: '2026-04-01 12:00:00', session_end: '2026-04-01 12:10:00', session_start_date: '2026-04-01', event_count: 2, platform: 'web'}
    silver_events_parsed:
      # Session A events
      - {event_id: 1, device_id: 1, user_id: null, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
      - {event_id: 2, device_id: 1, user_id: 100,  event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'}
      # Session B events — two signed-in observations
      - {event_id: 3, device_id: 2, user_id: 200, event_ts: '2026-04-01 11:02:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'}
      - {event_id: 4, device_id: 2, user_id: null, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
      - {event_id: 5, device_id: 2, user_id: 201, event_ts: '2026-04-01 11:08:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'}
      # Session C events — all anonymous
      - {event_id: 6, device_id: 3, user_id: null, event_ts: '2026-04-01 12:01:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
      - {event_id: 7, device_id: 3, user_id: null, event_ts: '2026-04-01 12:09:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
  expect:
    - {session_id: 'sa', forward_only_user_id: 100}
    - {session_id: 'sb', forward_only_user_id: 201}  # the LATER signed-in user wins, not the earlier (user_id 200)
    - {session_id: 'sc', forward_only_user_id: null}
---
