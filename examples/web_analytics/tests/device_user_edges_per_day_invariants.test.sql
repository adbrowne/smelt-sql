--- name: test_device_user_edges_per_day_invariants ---
materialization: test
test:
  model: device_user_edges
  inputs:
    silver_events_parsed:
      # Day 1 (2026-04-01): device 1 user 100 has 3 events; device 1 user 101 has 1; device 2 user 200 has 2
      - {event_id: 1, device_id: 1, user_id: 100, event_ts: '2026-04-01T09:00:00', event_date: '2026-04-01', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 2, device_id: 1, user_id: 100, event_ts: '2026-04-01T09:30:00', event_date: '2026-04-01', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 3, device_id: 1, user_id: 100, event_ts: '2026-04-01T10:00:00', event_date: '2026-04-01', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 4, device_id: 1, user_id: 101, event_ts: '2026-04-01T11:00:00', event_date: '2026-04-01', amplitude_id: 'u:101', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 5, device_id: 2, user_id: 200, event_ts: '2026-04-01T12:00:00', event_date: '2026-04-01', amplitude_id: 'u:200', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 6, device_id: 2, user_id: 200, event_ts: '2026-04-01T13:00:00', event_date: '2026-04-01', amplitude_id: 'u:200', event_name: 'view', platform: 'web', url: '/'}

      # Day 2 (2026-04-02): device 1 user 100 has 2 events; device 2 user 200 has 1
      - {event_id: 7, device_id: 1, user_id: 100, event_ts: '2026-04-02T09:00:00', event_date: '2026-04-02', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 8, device_id: 1, user_id: 100, event_ts: '2026-04-02T09:30:00', event_date: '2026-04-02', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 9, device_id: 2, user_id: 200, event_ts: '2026-04-02T10:00:00', event_date: '2026-04-02', amplitude_id: 'u:200', event_name: 'view', platform: 'web', url: '/'}

      # Anonymous events (user_id NULL) — must NOT appear in the edges output
      - {event_id: 10, device_id: 3, user_id: ~, event_ts: '2026-04-01T14:00:00', event_date: '2026-04-01', amplitude_id: 'd:3', event_name: 'view', platform: 'web', url: '/'}
      - {event_id: 11, device_id: 3, user_id: ~, event_ts: '2026-04-02T14:00:00', event_date: '2026-04-02', amplitude_id: 'd:3', event_name: 'view', platform: 'web', url: '/'}
  expect:
    # Day 1
    - {device_id: 1, user_id: 100, event_date: '2026-04-01', daily_event_count: 3, daily_first_seen: '2026-04-01T09:00:00', daily_last_seen: '2026-04-01T10:00:00'}
    - {device_id: 1, user_id: 101, event_date: '2026-04-01', daily_event_count: 1, daily_first_seen: '2026-04-01T11:00:00', daily_last_seen: '2026-04-01T11:00:00'}
    - {device_id: 2, user_id: 200, event_date: '2026-04-01', daily_event_count: 2, daily_first_seen: '2026-04-01T12:00:00', daily_last_seen: '2026-04-01T13:00:00'}
    # Day 2
    - {device_id: 1, user_id: 100, event_date: '2026-04-02', daily_event_count: 2, daily_first_seen: '2026-04-02T09:00:00', daily_last_seen: '2026-04-02T09:30:00'}
    - {device_id: 2, user_id: 200, event_date: '2026-04-02', daily_event_count: 1, daily_first_seen: '2026-04-02T10:00:00', daily_last_seen: '2026-04-02T10:00:00'}
---
