--- name: test_session_boundary_invariants ---
materialization: test
test:
  model: sessions
  inputs:
    silver_events_parsed:
      # device_id 1 — gap boundary: two events 35 minutes apart on the same
      # platform.  The 30-minute inactivity rule fires on the second event, so
      # two sessions are produced (session_seq 0 and 1, one event each).
      - {device_id: 1, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 1, event_ts: '2026-04-01 10:35:00', event_date: '2026-04-01', platform: 'web'}
      # device_id 2 — platform boundary: two events 5 minutes apart on different
      # platforms.  The platform-change rule fires on the second event, so two
      # sessions are produced even though the gap is only 5 minutes.
      - {device_id: 2, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 2, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', platform: 'ios'}
      # device_id 3 — no boundary: two events 5 minutes apart on the same
      # platform.  Neither rule fires, so both events belong to one session
      # (session_seq 0, event_count 2).
      - {device_id: 3, event_ts: '2026-04-01 12:00:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 3, event_ts: '2026-04-01 12:05:00', event_date: '2026-04-01', platform: 'web'}
  expect:
    # device_id 1: two sessions from the gap rule
    - {device_id: 1, session_seq: 0, event_count: 1, platform: 'web'}
    - {device_id: 1, session_seq: 1, event_count: 1, platform: 'web'}
    # device_id 2: two sessions from the platform-boundary rule
    - {device_id: 2, session_seq: 0, event_count: 1, platform: 'web'}
    - {device_id: 2, session_seq: 1, event_count: 1, platform: 'ios'}
    # device_id 3: one session (no boundary triggered)
    - {device_id: 3, session_seq: 0, event_count: 2, platform: 'web'}
---
