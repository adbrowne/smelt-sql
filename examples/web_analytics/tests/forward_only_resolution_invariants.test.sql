-- Forward-only identity resolution invariants.
-- Exercises the real gold/identity_forward_only model: the assertion query
-- selects from smelt.gold.identity_forward_only and PASSING mocks the model's
-- two external deps (silver.sessions, silver.events_deduped). The model's own
-- SQL runs as written against the mocked upstreams.
--
-- Invariants: the LAST signed-in event's user wins within a session; NULL when
-- a session has zero signed-in events (device fallback applied downstream).

smelt.test test_forward_only_resolution_invariants AS (
    SELECT session_id, forward_only_amplitude_id
    FROM smelt.gold.identity_forward_only
)
PASSING silver.sessions AS (
    -- Session A: one signed-in event at the end (user_id 100)
    {session_id: 'sa', device_id: 1, session_seq: 0, session_start: '2026-04-01 10:00:00', session_end: '2026-04-01 10:10:00', session_start_date: '2026-04-01', event_count: 2, platform: 'web'},
    -- Session B: two signed-in events; the LATER one (user_id 201) wins
    {session_id: 'sb', device_id: 2, session_seq: 0, session_start: '2026-04-01 11:00:00', session_end: '2026-04-01 11:10:00', session_start_date: '2026-04-01', event_count: 3, platform: 'web'},
    -- Session C: zero signed-in events; forward_only_amplitude_id stays NULL
    {session_id: 'sc', device_id: 3, session_seq: 0, session_start: '2026-04-01 12:00:00', session_end: '2026-04-01 12:10:00', session_start_date: '2026-04-01', event_count: 2, platform: 'web'}
)
PASSING silver.events_deduped AS (
    -- Session A events
    {event_id: 1, device_id: 1, user_id: null,  amplitude_id: 'd:1',   event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'},
    {event_id: 2, device_id: 1, user_id: 100,   amplitude_id: 'u:100', event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'},
    -- Session B events: two signed-in observations; the LATER (user 201) wins
    {event_id: 3, device_id: 2, user_id: 200,   amplitude_id: 'u:200', event_ts: '2026-04-01 11:02:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'},
    {event_id: 4, device_id: 2, user_id: null,  amplitude_id: 'd:2',   event_ts: '2026-04-01 11:05:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'},
    {event_id: 5, device_id: 2, user_id: 201,   amplitude_id: 'u:201', event_ts: '2026-04-01 11:08:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login'},
    -- Session C events: all anonymous
    {event_id: 6, device_id: 3, user_id: null,  amplitude_id: 'd:3',   event_ts: '2026-04-01 12:01:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'},
    {event_id: 7, device_id: 3, user_id: null,  amplitude_id: 'd:3',   event_ts: '2026-04-01 12:09:00', event_date: '2026-04-01', first_seen_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/'}
)
EXPECT (
    {session_id: 'sa', forward_only_amplitude_id: 'u:100'},
    {session_id: 'sb', forward_only_amplitude_id: 'u:201'},  -- the LATER signed-in user wins, not user_id 200
    {session_id: 'sc', forward_only_amplitude_id: null}      -- no signed-in events; device fallback applied downstream
)
