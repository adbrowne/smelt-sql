-- Session boundary invariants: 30-minute inactivity + platform-change rules,
-- plus the clock-anchored cut (`docs/research/20260711-clock-vs-root-anchored-sessions.md`
-- §"silver.sessions — clock-anchored cut"). Exercises the real
-- silver/sessions model (which sessionizes via the smelt.functions.sessionize
-- transparent function): the assertion query selects from smelt.silver.sessions
-- and PASSING mocks its single external dep (silver.events_deduped). The model
-- runs as written against the mocked events.
--
-- Six cases:
--   device 1 — gap boundary: 35-minute gap triggers new session (2 sessions of 1 event each)
--   device 2 — platform boundary: 5-minute gap + platform change triggers new session
--   device 3 — no boundary: 5-minute gap, same platform → one session of 2 events
--   device 4 — cross-midnight continuation: 20-minute gap straddling midnight → one session
--   device 5 — early-root cut: a continuous (25-minute-gap) chain rooted at
--     00:10 (time-of-day < 00:30, so the deadline is the *same* day's end)
--     is cut at that day's end — 58 events merge into the root's session,
--     and the first event past the deadline (00:20 the next day) roots its
--     own singleton session (58 + 1 = 59 events, conserved)
--   device 6 — late-root cut at the second midnight: a continuous
--     (25-minute-gap) chain rooted at 23:50 (time-of-day >= 00:30, so the
--     deadline reaches to the *next* day's end) crosses one midnight and is
--     cut at the second — 58 events merge into the root's session, and the
--     first two events past the deadline root a new session
--     (58 + 2 = 60 events, conserved)

smelt.test test_session_boundary_invariants AS (
    SELECT device_id, session_start, event_count, platform
    FROM smelt.silver.sessions
)
PASSING silver.events_deduped AS (
    -- device 1: gap boundary (35 minutes) → 2 sessions
    {device_id: 1, event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 1, event_ts: '2026-04-01 10:35:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    -- device 2: platform boundary (5 min, platform change web→ios) → 2 sessions
    {device_id: 2, event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 2, event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', platform: 'ios', utm_campaign: NULL},
    -- device 3: no boundary (5 min, same platform) → 1 session of 2 events
    {device_id: 3, event_ts: '2026-04-01 12:00:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 3, event_ts: '2026-04-01 12:05:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    -- device 4: cross-midnight continuation (20 min, same platform) → 1 session
    {device_id: 4, event_ts: '2026-04-01 23:50:00', event_date: '2026-04-01', platform: 'web', utm_campaign: NULL},
    {device_id: 4, event_ts: '2026-04-02 00:10:00', event_date: '2026-04-02', platform: 'web', utm_campaign: NULL},
    -- device 5: early-root cut — a continuous 25-minute-gap chain rooted at
    -- 00:10 (< 00:30); the deadline is the same day's end, so the chain is
    -- cut there and the first post-deadline event roots its own session.
    {device_id: 5, event_ts: '2026-04-03 00:10:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 00:35:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 01:00:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 01:25:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 01:50:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 02:15:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 02:40:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 03:05:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 03:30:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 03:55:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 04:20:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 04:45:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 05:10:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 05:35:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 06:00:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 06:25:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 06:50:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 07:15:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 07:40:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 08:05:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 08:30:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 08:55:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 09:20:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 09:45:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 10:10:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 10:35:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 11:00:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 11:25:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 11:50:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 12:15:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 12:40:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 13:05:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 13:30:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 13:55:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 14:20:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 14:45:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 15:10:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 15:35:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 16:00:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 16:25:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 16:50:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 17:15:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 17:40:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 18:05:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 18:30:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 18:55:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 19:20:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 19:45:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 20:10:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 20:35:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 21:00:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 21:25:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 21:50:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 22:15:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 22:40:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 23:05:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 23:30:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-03 23:55:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 5, event_ts: '2026-04-04 00:20:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    -- device 6: late-root cut at the second midnight — a continuous
    -- 25-minute-gap chain rooted at 23:50 (>= 00:30); the deadline reaches
    -- to the *next* day's end, so the chain crosses one midnight before
    -- being cut, and the first two post-deadline events root a new session.
    {device_id: 6, event_ts: '2026-04-03 23:50:00', event_date: '2026-04-03', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 00:15:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 00:40:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 01:05:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 01:30:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 01:55:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 02:20:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 02:45:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 03:10:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 03:35:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 04:00:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 04:25:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 04:50:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 05:15:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 05:40:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 06:05:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 06:30:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 06:55:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 07:20:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 07:45:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 08:10:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 08:35:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 09:00:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 09:25:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 09:50:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 10:15:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 10:40:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 11:05:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 11:30:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 11:55:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 12:20:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 12:45:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 13:10:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 13:35:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 14:00:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 14:25:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 14:50:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 15:15:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 15:40:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 16:05:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 16:30:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 16:55:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 17:20:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 17:45:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 18:10:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 18:35:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 19:00:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 19:25:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 19:50:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 20:15:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 20:40:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 21:05:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 21:30:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 21:55:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 22:20:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 22:45:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 23:10:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-04 23:35:00', event_date: '2026-04-04', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-05 00:00:00', event_date: '2026-04-05', platform: 'web', utm_campaign: NULL},
    {device_id: 6, event_ts: '2026-04-05 00:25:00', event_date: '2026-04-05', platform: 'web', utm_campaign: NULL}
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
    {device_id: 4, session_start: '2026-04-01T23:50:00', event_count: 2, platform: 'web'},
    -- device 5: early-root cut — 58 events merge into the root's session
    -- (cut at its own day's end), the 59th roots its own singleton session
    {device_id: 5, session_start: '2026-04-03T00:10:00', event_count: 58, platform: 'web'},
    {device_id: 5, session_start: '2026-04-04T00:20:00', event_count: 1, platform: 'web'},
    -- device 6: late-root cut at the second midnight — 58 events merge into
    -- the root's session (crossing one midnight), the last 2 root a new session
    {device_id: 6, session_start: '2026-04-03T23:50:00', event_count: 58, platform: 'web'},
    {device_id: 6, session_start: '2026-04-05T00:00:00', event_count: 2, platform: 'web'}
)
