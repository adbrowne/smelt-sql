-- DAU monotonicity invariants across four identity methods.
-- Exercises the real marts/daily_active_users_by_method model: the assertion
-- query selects from smelt.marts.daily_active_users_by_method and PASSING mocks
-- its single external dep (gold.eventstream_with_identity). The model's per-day
-- aggregation runs as written, surfacing the expected ordering relationships
-- between raw / forward-only / backward-fill / connected-components.

smelt.test test_dau_monotonicity_invariants AS (
    SELECT
        event_date,
        total_events,
        dau_raw,
        dau_forward_only,
        dau_backward_fill,
        dau_connected_components,
        identified_events_raw,
        identified_events_forward_only,
        identified_events_backward_fill,
        identified_events_connected_components
    FROM smelt.marts.daily_active_users_by_method
)
PASSING gold.eventstream_with_identity AS (
    -- Day 1 (2026-04-01): 4 events on 2 devices.
    -- Device 1 has one signed-in event (user 100); backward_fill back-tags every
    -- event on device 1 to 'u:100'. Device 2 stays anonymous everywhere.
    {event_id: 1, device_id: 1, event_user_id: null, amplitude_id: 'd:1',   event_ts: '2026-04-01 10:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sa', forward_only_amplitude_id: 'u:100', backward_fill_amplitude_id: 'u:100', connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'},
    {event_id: 2, device_id: 1, event_user_id: 100,  amplitude_id: 'u:100', event_ts: '2026-04-01 10:05:00', event_date: '2026-04-01', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sa', forward_only_amplitude_id: 'u:100', backward_fill_amplitude_id: 'u:100', connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'},
    {event_id: 3, device_id: 1, event_user_id: null, amplitude_id: 'd:1',   event_ts: '2026-04-01 10:08:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sa', forward_only_amplitude_id: 'd:1',   backward_fill_amplitude_id: 'u:100', connected_components_amplitude_id: 'u:100', connected_components_cluster_id: 'u:100'},
    {event_id: 4, device_id: 2, event_user_id: null, amplitude_id: 'd:2',   event_ts: '2026-04-01 11:00:00', event_date: '2026-04-01', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sb', forward_only_amplitude_id: 'd:2',   backward_fill_amplitude_id: 'd:2',   connected_components_amplitude_id: 'd:2',   connected_components_cluster_id: 'd:2'},
    -- Day 2 (2026-04-02): 4 events on 2 devices, both signed-in.
    -- Devices 3 and 4 are in cluster {200, 201} with representative 'u:200' (cluster collapse).
    {event_id: 5, device_id: 3, event_user_id: 200,  amplitude_id: 'u:200', event_ts: '2026-04-02 10:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sc', forward_only_amplitude_id: 'u:200', backward_fill_amplitude_id: 'u:200', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'},
    {event_id: 6, device_id: 3, event_user_id: null, amplitude_id: 'd:3',   event_ts: '2026-04-02 10:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sc', forward_only_amplitude_id: 'u:200', backward_fill_amplitude_id: 'u:200', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'},
    {event_id: 7, device_id: 4, event_user_id: 201,  amplitude_id: 'u:201', event_ts: '2026-04-02 11:00:00', event_date: '2026-04-02', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'sd', forward_only_amplitude_id: 'u:201', backward_fill_amplitude_id: 'u:201', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'},
    {event_id: 8, device_id: 4, event_user_id: null, amplitude_id: 'd:4',   event_ts: '2026-04-02 11:05:00', event_date: '2026-04-02', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'sd', forward_only_amplitude_id: 'u:201', backward_fill_amplitude_id: 'u:201', connected_components_amplitude_id: 'u:200', connected_components_cluster_id: 'u:200'},
    -- Day 3 (2026-04-03): 2 events on 1 device, 1 signed-in (user 300) + 1 anonymous.
    {event_id: 9,  device_id: 5, event_user_id: 300,  amplitude_id: 'u:300', event_ts: '2026-04-03 10:00:00', event_date: '2026-04-03', event_name: 'login',     platform: 'web', url: 'https://example.com/login', session_id: 'se', forward_only_amplitude_id: 'u:300', backward_fill_amplitude_id: 'u:300', connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'},
    {event_id: 10, device_id: 5, event_user_id: null, amplitude_id: 'd:5',   event_ts: '2026-04-03 10:05:00', event_date: '2026-04-03', event_name: 'page_view', platform: 'web', url: 'https://example.com/',      session_id: 'se', forward_only_amplitude_id: 'd:5',   backward_fill_amplitude_id: 'u:300', connected_components_amplitude_id: 'u:300', connected_components_cluster_id: 'u:300'}
)
EXPECT (
    {event_date: '2026-04-01', total_events: 4, dau_raw: 3, dau_forward_only: 3, dau_backward_fill: 2, dau_connected_components: 2, identified_events_raw: 1, identified_events_forward_only: 2, identified_events_backward_fill: 3, identified_events_connected_components: 3},
    {event_date: '2026-04-02', total_events: 4, dau_raw: 4, dau_forward_only: 2, dau_backward_fill: 2, dau_connected_components: 1, identified_events_raw: 2, identified_events_forward_only: 4, identified_events_backward_fill: 4, identified_events_connected_components: 4},
    {event_date: '2026-04-03', total_events: 2, dau_raw: 2, dau_forward_only: 2, dau_backward_fill: 1, dau_connected_components: 1, identified_events_raw: 1, identified_events_forward_only: 1, identified_events_backward_fill: 2, identified_events_connected_components: 2}
)
