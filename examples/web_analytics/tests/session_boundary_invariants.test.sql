-- Session boundary invariants: 30-minute inactivity + platform-change rules.
-- Exercises the real silver/sessions model (which sessionizes via the
-- smelt.functions.sessionize transparent function): the assertion query selects
-- from smelt.silver.sessions and PASSING mocks its single external dep
-- (silver.events_parsed). The model runs as written against the mocked events.
--
-- Four cases:
--   device 1 — gap boundary: 35-minute gap triggers new session (2 sessions of 1 event each)
--   device 2 — platform boundary: 5-minute gap + platform change triggers new session
--   device 3 — no boundary: 5-minute gap, same platform → one session of 2 events
--   device 4 — cross-midnight continuation: 20-minute gap straddling midnight → one session

smelt.test test_session_boundary_invariants AS (
    SELECT device_id, session_start, event_count, platform
    FROM smelt.silver.sessions
)
PASSING silver.events_parsed AS (
    -- device 1: gap boundary (35 minutes) → 2 sessions
    {device_id: 1, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', platform: 'web'},
    {device_id: 1, event_ts: '2026-04-01 10:35:00', event_date: '2026-04-01', platform: 'web'},
    -- device 2: platform boundary (5 min, platform change web→ios) → 2 sessions
    {device_id: 2, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', platform: 'web'},
    {device_id: 2, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', platform: 'ios'},
    -- device 3: no boundary (5 min, same platform) → 1 session of 2 events
    {device_id: 3, event_ts: '2026-04-01 12:00:00', event_date: '2026-04-01', platform: 'web'},
    {device_id: 3, event_ts: '2026-04-01 12:05:00', event_date: '2026-04-01', platform: 'web'},
    -- device 4: cross-midnight continuation (20 min, same platform) → 1 session
    {device_id: 4, event_ts: '2026-04-01 23:50:00', event_date: '2026-04-01', platform: 'web'},
    {device_id: 4, event_ts: '2026-04-02 00:10:00', event_date: '2026-04-02', platform: 'web'}
)
EXPECT (
    -- device 1: two sessions from the gap rule; each has 1 event
    {device_id: 1, session_start: '2026-04-01T10:00:00', event_count: 1, platform: 'web'},
    {device_id: 1, session_start: '2026-04-01T10:35:00', event_count: 1, platform: 'web'},
    -- device 2: two sessions from the platform-boundary rule
    {device_id: 2, session_start: '2026-04-01T11:00:00', event_count: 1, platform: 'web'},
    {device_id: 2, session_start: '2026-04-01T11:05:00', event_count: 1, platform: 'ios'},
    -- device 3: one session (no boundary triggered)
    {device_id: 3, session_start: '2026-04-01T12:00:00', event_count: 2, platform: 'web'},
    -- device 4: one cross-midnight session; both events merge into a single session
    {device_id: 4, session_start: '2026-04-01T23:50:00', event_count: 2, platform: 'web'}
)
