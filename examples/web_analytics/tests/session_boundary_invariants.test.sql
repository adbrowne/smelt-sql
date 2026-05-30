--- name: test_session_boundary_invariants ---
materialization: test
test:
  model: sessions
  inputs:
    silver_events_parsed:
      # device_id 1 — gap boundary: two events 35 minutes apart on the same
      # platform. The 30-minute inactivity rule fires on the second event, so
      # two sessions are produced (one event each), keyed by their start ts.
      - {device_id: 1, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 1, event_ts: '2026-04-01 10:35:00', event_date: '2026-04-01', platform: 'web'}
      # device_id 2 — platform boundary: two events 5 minutes apart on different
      # platforms. The platform-change rule fires on the second event, so two
      # sessions are produced even though the gap is only 5 minutes.
      - {device_id: 2, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 2, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', platform: 'ios'}
      # device_id 3 — no boundary: two events 5 minutes apart on the same
      # platform. Neither rule fires, so both events belong to one session.
      - {device_id: 3, event_ts: '2026-04-01 12:00:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 3, event_ts: '2026-04-01 12:05:00', event_date: '2026-04-01', platform: 'web'}
      # device_id 4 — cross-midnight continuation: two events 20 minutes apart
      # straddling midnight on the same platform. Neither rule fires, so they are
      # ONE session whose session_start_date is the earlier day (2026-04-01).
      - {device_id: 4, event_ts: '2026-04-01 23:50:00', event_date: '2026-04-01', platform: 'web'}
      - {device_id: 4, event_ts: '2026-04-02 00:10:00', event_date: '2026-04-02', platform: 'web'}
  expect:
    # device_id 1: two sessions from the gap rule, keyed by session_start
    - {device_id: 1, session_start: '2026-04-01T10:00:00', event_count: 1, platform: 'web'}
    - {device_id: 1, session_start: '2026-04-01T10:35:00', event_count: 1, platform: 'web'}
    # device_id 2: two sessions from the platform-boundary rule
    - {device_id: 2, session_start: '2026-04-01T11:00:00', event_count: 1, platform: 'web'}
    - {device_id: 2, session_start: '2026-04-01T11:05:00', event_count: 1, platform: 'ios'}
    # device_id 3: one session (no boundary triggered)
    - {device_id: 3, session_start: '2026-04-01T12:00:00', event_count: 2, platform: 'web'}
    # device_id 4: one cross-midnight session — both events merge into a single
    # session whose start (and therefore session_start_date) is on the first day.
    - {device_id: 4, session_start: '2026-04-01T23:50:00', event_count: 2, platform: 'web'}
---
