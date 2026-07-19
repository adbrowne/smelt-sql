-- Cumulative device-user edge invariants.
-- Exercises the real silver/device_user_edges model: the assertion query
-- selects from smelt.silver.device_user_edges and PASSING mocks its single
-- external dep (silver.events_deduped). The model's aggregation runs as written:
-- event_count sums; first_seen is the global MIN; last_seen is the global MAX;
-- anonymous events (user_id IS NULL) must not appear.

smelt.test test_device_user_edges_cumulative_invariants AS (
    SELECT device_id, user_id, event_count, first_seen, last_seen
    FROM smelt.silver.device_user_edges
)
PASSING silver.events_deduped AS (
    -- Day 1: device 1 user 100 has 3 events; device 1 user 101 has 1; device 2 user 200 has 2
    {event_id: 1, device_id: 1, user_id: 100, event_ts: '2026-04-01T09:00:00', event_date: '2026-04-01', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 2, device_id: 1, user_id: 100, event_ts: '2026-04-01T09:30:00', event_date: '2026-04-01', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 3, device_id: 1, user_id: 100, event_ts: '2026-04-01T10:00:00', event_date: '2026-04-01', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 4, device_id: 1, user_id: 101, event_ts: '2026-04-01T11:00:00', event_date: '2026-04-01', amplitude_id: 'u:101', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 5, device_id: 2, user_id: 200, event_ts: '2026-04-01T12:00:00', event_date: '2026-04-01', amplitude_id: 'u:200', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 6, device_id: 2, user_id: 200, event_ts: '2026-04-01T13:00:00', event_date: '2026-04-01', amplitude_id: 'u:200', event_name: 'view', platform: 'web', url: '/'},
    -- Day 2: device 1 user 100 has 2 events; device 2 user 200 has 1
    {event_id: 7, device_id: 1, user_id: 100, event_ts: '2026-04-02T09:00:00', event_date: '2026-04-02', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 8, device_id: 1, user_id: 100, event_ts: '2026-04-02T09:30:00', event_date: '2026-04-02', amplitude_id: 'u:100', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 9, device_id: 2, user_id: 200, event_ts: '2026-04-02T10:00:00', event_date: '2026-04-02', amplitude_id: 'u:200', event_name: 'view', platform: 'web', url: '/'},
    -- Anonymous events (user_id NULL) — must NOT appear in the edges output
    {event_id: 10, device_id: 3, user_id: null, event_ts: '2026-04-01T14:00:00', event_date: '2026-04-01', amplitude_id: 'd:3', event_name: 'view', platform: 'web', url: '/'},
    {event_id: 11, device_id: 3, user_id: null, event_ts: '2026-04-02T14:00:00', event_date: '2026-04-02', amplitude_id: 'd:3', event_name: 'view', platform: 'web', url: '/'}
)
EXPECT (
    -- One row per (device, user) — cumulative across all dates.
    {device_id: 1, user_id: 100, event_count: 5, first_seen: '2026-04-01T09:00:00', last_seen: '2026-04-02T09:30:00'},
    {device_id: 1, user_id: 101, event_count: 1, first_seen: '2026-04-01T11:00:00', last_seen: '2026-04-01T11:00:00'},
    {device_id: 2, user_id: 200, event_count: 3, first_seen: '2026-04-01T12:00:00', last_seen: '2026-04-02T10:00:00'}
)
